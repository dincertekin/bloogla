//! Importing a site from WordPress (Settings → Import & export).
//!
//! Two steps, like restoring a backup: [`check`] reads the uploaded export
//! file and shows what's in it, without changing anything; [`start`] then
//! saves a backup of the site and brings the posts, pages, tags, images
//! and comments in, in the background, so a big site can take its time.
//! The screen asks [`now`] for the progress and, at the end, the report.

pub mod images;
pub mod wordpress;

use crate::app::state::AppState;
use crate::content::text::{slugify, url_decode};
use crate::db::{or_log, posts, tags};
use std::collections::HashMap;
use std::path::Path;
use std::sync::Mutex;
use wordpress::{Export, Item};

/// The largest export file accepted. Exports hold text, not images, so even
/// big sites stay well below this.
pub const MAX_FILE_BYTES: u64 = 200 << 20;

pub const BUSY: &str = "An import is already running. Wait for it to finish.";
pub const CHECK_AGAIN: &str = "Check the file again first.";
const BACKUP_FAILED: &str = "Couldn't save a backup of your site as it is now, so nothing was imported. Check that there's enough free disk space, then try again.";

/// Where the import is. There's one for the whole site.
#[derive(Default)]
enum Stage {
    #[default]
    Idle,
    Checked(Box<Checked>),
    Running(Progress),
    Done(Report),
}

static STAGE: Mutex<Stage> = Mutex::new(Stage::Idle);

/// What the screen shows: a copy of the stage, without the file's contents.
#[derive(Clone)]
pub enum View {
    Idle,
    Checked(Summary),
    Running(Progress),
    Done(Report),
}

/// A checked file, waiting for "Import".
struct Checked {
    export: Export,
    summary: Summary,
}

/// What a checked file holds.
#[derive(Clone, Default)]
pub struct Summary {
    pub site_title: String,
    /// The old site, like `ayseskitchen.com`.
    pub site: String,
    pub posts: i64,
    pub pages: i64,
    pub tags: i64,
    /// Approved comments.
    pub comments: i64,
    /// Images on the old site that posts show.
    pub images: i64,
    pub authors: Vec<AuthorMatch>,
    /// Titles of posts whose address is already used here (skipped).
    pub taken: Vec<String>,
    /// One old address, to show how they keep working.
    pub example_address: Option<String>,
}

/// Someone who wrote on the old site, and who they are here.
#[derive(Clone)]
pub struct AuthorMatch {
    pub name: String,
    pub email: String,
    pub posts: i64,
    /// The name of the person here with the same email, if there is one.
    pub found: Option<String>,
}

/// How far a running import is.
#[derive(Clone, Default)]
pub struct Progress {
    pub site_title: String,
    pub step: Step,
    pub posts: i64,
    pub pages: i64,
    pub tags: i64,
    pub images_done: i64,
    pub images_total: i64,
    pub copy_images: bool,
    pub bring_comments: bool,
}

#[derive(Clone, Copy, Default, PartialEq, PartialOrd)]
pub enum Step {
    #[default]
    Backup,
    Posts,
    Images,
    Comments,
}

impl Progress {
    /// Posts and pages brought in so far.
    pub fn posts_and_pages(&self) -> i64 {
        self.posts + self.pages
    }
}

/// What an import did.
#[derive(Clone, Default)]
pub struct Report {
    pub site_title: String,
    pub posts: i64,
    pub pages: i64,
    pub tags: i64,
    pub comments: i64,
    pub images: i64,
    pub failed_images: Vec<FailedImage>,
    /// Titles of posts skipped because their address was already used.
    pub skipped: Vec<String>,
    /// Set when the import stopped before it began (English).
    pub error: Option<&'static str>,
}

impl Report {
    /// How many images stayed where they were.
    pub fn failed_count(&self) -> i64 {
        self.failed_images.len() as i64
    }
}

/// An image that stayed on the old site.
#[derive(Clone)]
pub struct FailedImage {
    pub post: String,
    /// The address without `https://`, shorter to read.
    pub address: String,
    /// One of the reasons in [`images`] (English).
    pub reason: &'static str,
}

/// The stage, for the screen.
pub fn now() -> View {
    match &*stage() {
        Stage::Idle => View::Idle,
        Stage::Checked(checked) => View::Checked(checked.summary.clone()),
        Stage::Running(progress) => View::Running(progress.clone()),
        Stage::Done(report) => View::Done(report.clone()),
    }
}

fn stage() -> std::sync::MutexGuard<'static, Stage> {
    // A panic while the lock was held leaves it "poisoned"; the stage inside
    // is still fine to use.
    STAGE
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

/// Whether an import is running (then nothing else may start).
pub fn is_running() -> bool {
    matches!(*stage(), Stage::Running(_))
}

/// Forget a checked file or a finished import's report.
pub fn reset() {
    let mut stage = stage();
    if !matches!(*stage, Stage::Running(_)) {
        *stage = Stage::Idle;
    }
}

/// Step 1: read the export file at `file` and see what's in it and what it
/// would do here. Nothing changes on the site.
pub async fn check(state: &AppState, file: &Path) -> Result<(), &'static str> {
    if is_running() {
        return Err(BUSY);
    }
    let bytes = tokio::fs::read(file)
        .await
        .map_err(|_| wordpress::NOT_WORDPRESS)?;
    let export = tokio::task::spawn_blocking(move || wordpress::read(&bytes))
        .await
        .map_err(|_| wordpress::DAMAGED)??;
    let summary = summarize(state, &export).await;
    *stage() = Stage::Checked(Box::new(Checked { export, summary }));
    Ok(())
}

/// What WordPress's status becomes here; `None` for items Bloogla skips
/// (the trash, unfinished autosaves).
fn status(item: &Item) -> Option<&'static str> {
    match item.status.as_str() {
        "publish" => Some("published"),
        "future" => Some("scheduled"),
        "draft" | "pending" | "private" => Some("draft"),
        _ => None,
    }
}

/// The posts and pages of an export that are brought in.
fn importable(export: &Export) -> impl Iterator<Item = &Item> {
    export
        .items
        .iter()
        .filter(|item| matches!(item.kind.as_str(), "post" | "page") && status(item).is_some())
}

/// The address a post gets here: its WordPress one when that's usable.
fn slug_for(item: &Item) -> String {
    let wordpress_slug = url_decode(&item.slug);
    if wordpress_slug.trim().is_empty() {
        slugify(&item.title)
    } else {
        slugify(&wordpress_slug)
    }
}

/// The site an export comes from, like `ayseskitchen.com`.
fn site_of(export: &Export) -> String {
    images::site_of(&export.site_url).unwrap_or_default()
}

/// Every image on the old site a post shows: in its text, and its cover.
fn images_of<'a>(item: &'a Item, covers: &'a HashMap<i64, String>) -> Vec<String> {
    let mut found = images::find(&item.content);
    if let Some(cover) = item.cover_id.and_then(|id| covers.get(&id)) {
        if !found.contains(cover) {
            found.push(cover.clone());
        }
    }
    found
}

/// The addresses of uploaded images, by their WordPress id.
fn covers(export: &Export) -> HashMap<i64, String> {
    export
        .items
        .iter()
        .filter(|item| item.kind == "attachment" && !item.attachment_url.is_empty())
        .map(|item| (item.wordpress_id, item.attachment_url.clone()))
        .collect()
}

async fn summarize(state: &AppState, export: &Export) -> Summary {
    let site = site_of(export);
    let covers = covers(export);
    let mut summary = Summary {
        site_title: export.site_title.clone(),
        site: site.clone(),
        ..Summary::default()
    };
    let mut tag_names: Vec<&str> = Vec::new();
    let mut images_on_site: Vec<String> = Vec::new();
    let mut slugs_in_file: Vec<String> = Vec::new();
    let mut posts_by_login: HashMap<&str, i64> = HashMap::new();

    for item in importable(export) {
        let slug = slug_for(item);
        if slugs_in_file.contains(&slug) || is_taken(state, &slug).await {
            summary.taken.push(item.title.clone());
            continue;
        }
        slugs_in_file.push(slug);
        if item.kind == "page" {
            summary.pages += 1;
        } else {
            summary.posts += 1;
        }
        if summary.example_address.is_none() && !item.link.is_empty() {
            summary.example_address = Some(without_scheme(&item.link));
        }
        *posts_by_login.entry(&item.author_login).or_default() += 1;
        for tag in &item.tags {
            if !tag_names.contains(&tag.as_str()) {
                tag_names.push(tag);
            }
        }
        summary.comments += item.comments.iter().filter(|c| brings(c)).count() as i64;
        for url in images_of(item, &covers) {
            let full = images::full_size(&url);
            if images::site_of(&url).as_deref() == Some(site.as_str())
                && !images_on_site.contains(&full)
            {
                images_on_site.push(full);
            }
        }
    }
    summary.tags = tag_names.len() as i64;
    summary.images = images_on_site.len() as i64;

    for author in &export.authors {
        let posts = posts_by_login
            .get(author.login.as_str())
            .copied()
            .unwrap_or(0);
        if posts == 0 {
            continue;
        }
        let found: Option<String> = or_log(
            sqlx::query_scalar("SELECT name FROM users WHERE email = ? COLLATE NOCASE")
                .bind(&author.email)
                .fetch_optional(&state.pool)
                .await,
            "match imported author",
        );
        summary.authors.push(AuthorMatch {
            name: if author.name.is_empty() {
                author.login.clone()
            } else {
                author.name.clone()
            },
            email: author.email.clone(),
            posts,
            found,
        });
    }
    summary
}

/// Whether a post here already uses `slug` (or it's one Bloogla keeps for
/// itself, like `admin`).
async fn is_taken(state: &AppState, slug: &str) -> bool {
    match posts::unique_slug(&state.pool, slug, None).await {
        Ok(free) => free != slug,
        Err(_) => true,
    }
}

/// Comments that come along: approved ones, not links from other sites.
fn brings(comment: &wordpress::Comment) -> bool {
    comment.approved && matches!(comment.kind.as_str(), "" | "comment")
}

fn without_scheme(url: &str) -> String {
    url.split_once("://")
        .map_or(url, |(_, rest)| rest)
        .to_string()
}

/// Step 2: start importing the checked file in the background. Posts whose
/// author isn't found here go to `owner_id` (the person importing).
pub fn start(
    state: AppState,
    owner_id: i64,
    copy_images: bool,
    bring_comments: bool,
) -> Result<(), &'static str> {
    let checked = {
        let mut stage = stage();
        match std::mem::take(&mut *stage) {
            Stage::Checked(checked) => {
                *stage = Stage::Running(Progress {
                    site_title: checked.summary.site_title.clone(),
                    copy_images,
                    bring_comments,
                    ..Progress::default()
                });
                checked
            }
            other => {
                let error = if matches!(other, Stage::Running(_)) {
                    BUSY
                } else {
                    CHECK_AGAIN
                };
                *stage = other;
                return Err(error);
            }
        }
    };
    tokio::spawn(async move {
        let report = run(
            &state,
            checked.export,
            owner_id,
            copy_images,
            bring_comments,
        )
        .await;
        tracing::info!(
            "Imported {} posts, {} pages and {} images from WordPress",
            report.posts,
            report.pages,
            report.images
        );
        *stage() = Stage::Done(report);
    });
    Ok(())
}

/// Change the progress the screen shows.
fn progress(change: impl FnOnce(&mut Progress)) {
    if let Stage::Running(progress) = &mut *stage() {
        change(progress);
    }
}

/// A post brought in, for the images and comments steps.
struct Imported<'a> {
    id: i64,
    item: &'a Item,
    content: String,
}

async fn run(
    state: &AppState,
    export: Export,
    owner_id: i64,
    copy_images: bool,
    bring_comments: bool,
) -> Report {
    let mut report = Report {
        site_title: export.site_title.clone(),
        ..Report::default()
    };

    // A backup first, like before a restore: this is the way back.
    let folder = &state.config.folder;
    let stamp = chrono::Utc::now().format("%Y%m%d-%H%M%S");
    let safety = folder
        .join(super::backup::BACKUP_DIR)
        .join(format!("before-import-{stamp}.zip"));
    let saved = match safety.parent().map(std::fs::create_dir_all) {
        Some(Err(e)) => Err(e.to_string()),
        _ => super::backup::write_backup(&state.pool, &folder.join("uploads"), &safety).await,
    };
    if let Err(e) = saved {
        tracing::error!("Import stopped, the safety backup failed: {e}");
        report.error = Some(BACKUP_FAILED);
        return report;
    }

    // Posts, pages and their tags.
    progress(|p| p.step = Step::Posts);
    let authors = authors_here(state, &export, owner_id).await;
    let covers = covers(&export);
    let mut imported: Vec<Imported> = Vec::new();
    let mut tag_names: Vec<String> = Vec::new();
    for item in importable(&export) {
        let slug = slug_for(item);
        if is_taken(state, &slug).await {
            report.skipped.push(item.title.clone());
            continue;
        }
        let author_id = authors.get(&item.author_login).copied().unwrap_or(owner_id);
        let cover = if copy_images {
            None // set once the image is copied
        } else {
            item.cover_id.and_then(|id| covers.get(&id)).cloned()
        };
        match insert_post(state, item, &slug, author_id, cover.as_deref()).await {
            Ok(id) => {
                if item.kind == "page" {
                    report.pages += 1;
                } else {
                    report.posts += 1;
                }
                for tag in &item.tags {
                    if !tag_names.contains(tag) {
                        tag_names.push(tag.clone());
                    }
                }
                let (posts_now, pages_now, tags_now) =
                    (report.posts, report.pages, tag_names.len() as i64);
                progress(|p| {
                    p.posts = posts_now;
                    p.pages = pages_now;
                    p.tags = tags_now;
                });
                imported.push(Imported {
                    id,
                    item,
                    content: wordpress::without_block_markers(&item.content),
                });
            }
            Err(e) => {
                tracing::error!("Couldn't import “{}”: {e}", item.title);
                report.skipped.push(item.title.clone());
            }
        }
    }
    report.tags = tag_names.len() as i64;

    if copy_images {
        progress(|p| p.step = Step::Images);
        copy_all_images(
            state,
            &site_of(&export),
            &covers,
            &mut imported,
            &mut report,
        )
        .await;
    }

    if bring_comments {
        progress(|p| p.step = Step::Comments);
        for post in &imported {
            for comment in post.item.comments.iter().filter(|c| brings(c)) {
                if insert_comment(state, post.id, comment).await {
                    report.comments += 1;
                }
            }
        }
    }
    report
}

/// The people here, by their WordPress user name: matched by email.
async fn authors_here(state: &AppState, export: &Export, owner_id: i64) -> HashMap<String, i64> {
    let mut here = HashMap::new();
    for author in &export.authors {
        let id: Option<i64> = or_log(
            sqlx::query_scalar("SELECT id FROM users WHERE email = ? COLLATE NOCASE")
                .bind(&author.email)
                .fetch_optional(&state.pool)
                .await,
            "match imported author",
        );
        here.insert(author.login.clone(), id.unwrap_or(owner_id));
    }
    here
}

/// Save one post or page with its tags. Gives back its new id.
async fn insert_post(
    state: &AppState,
    item: &Item,
    slug: &str,
    author_id: i64,
    cover: Option<&str>,
) -> Result<i64, sqlx::Error> {
    let title = if item.title.trim().is_empty() {
        slug.replace('-', " ")
    } else {
        item.title.trim().to_string()
    };
    let content = wordpress::without_block_markers(&item.content);
    let id = sqlx::query(
        "INSERT INTO posts (title, slug, content, cover_image, status, published_at, is_page,
                            author_id)
         VALUES (?, ?, ?, ?, ?, ?, ?, ?)",
    )
    .bind(&title)
    .bind(slug)
    .bind(&content)
    .bind(cover)
    .bind(status(item).unwrap_or("draft"))
    .bind(publish_date(item))
    .bind(item.kind == "page")
    .bind(author_id)
    .execute(&state.pool)
    .await?
    .last_insert_rowid();

    let tag_ids = tags::with_new(&state.pool, &[], &[item.tags.join(",")]).await;
    tags::set_for_post(&state.pool, id, &tag_ids).await;

    // A WordPress address Bloogla had to change (`künefe` → `kunefe`) still
    // leads here: old links end with it (see `site::pages::fallback`).
    let old_slug = url_decode(&item.slug);
    if !old_slug.is_empty() && old_slug != slug {
        let _ =
            sqlx::query("INSERT OR IGNORE INTO slug_redirects (old_slug, post_id) VALUES (?, ?)")
                .bind(&old_slug)
                .bind(id)
                .execute(&state.pool)
                .await;
    }
    Ok(id)
}

/// When it was published, in UTC (`2024-05-01 09:30:00`). Drafts have no
/// UTC date in WordPress, so their local one is used, or now.
fn publish_date(item: &Item) -> String {
    let parses = |date: &str| {
        chrono::NaiveDateTime::parse_from_str(date, "%Y-%m-%d %H:%M:%S")
            .ok()
            .map(|d| d.format("%Y-%m-%d %H:%M:%S").to_string())
    };
    parses(&item.date_utc)
        .or_else(|| parses(&item.date_local))
        .unwrap_or_else(|| chrono::Utc::now().format("%Y-%m-%d %H:%M:%S").to_string())
}

async fn insert_comment(state: &AppState, post_id: i64, comment: &wordpress::Comment) -> bool {
    let author = if comment.author.trim().is_empty() {
        "Anonymous"
    } else {
        comment.author.trim()
    };
    let date = chrono::NaiveDateTime::parse_from_str(&comment.date_utc, "%Y-%m-%d %H:%M:%S")
        .unwrap_or_else(|_| chrono::Utc::now().naive_utc());
    sqlx::query(
        "INSERT INTO comments (post_id, author_name, author_email, content, status, created_at)
         VALUES (?, ?, ?, ?, 'approved', ?)",
    )
    .bind(post_id)
    .bind(author)
    .bind(comment.email.trim())
    .bind(comment.content.trim())
    .bind(date.format("%Y-%m-%d %H:%M:%S").to_string())
    .execute(&state.pool)
    .await
    .is_ok()
}

/// Copy every image the imported posts show from the old site into Media,
/// and point the posts at the copies. Images on other sites, or ones that
/// fail, stay where they are and are listed in the report.
async fn copy_all_images(
    state: &AppState,
    site: &str,
    covers: &HashMap<i64, String>,
    imported: &mut [Imported<'_>],
    report: &mut Report,
) {
    // Each full-size image once, with the posts' addresses for it.
    let mut wanted: Vec<String> = Vec::new();
    let mut addresses: HashMap<String, Vec<String>> = HashMap::new();
    for post in imported.iter() {
        for url in images_of(post.item, covers) {
            let full = images::full_size(&url);
            if images::site_of(&url).as_deref() != Some(site) {
                // On another site: not copied, but said so once.
                if !report
                    .failed_images
                    .iter()
                    .any(|f| f.address == without_scheme(&url))
                {
                    report.failed_images.push(FailedImage {
                        post: post.item.title.clone(),
                        address: without_scheme(&url),
                        reason: images::OTHER_SITE,
                    });
                }
                continue;
            }
            if !wanted.contains(&full) {
                wanted.push(full.clone());
            }
            let list = addresses.entry(full).or_default();
            if !list.contains(&url) {
                list.push(url);
            }
        }
    }
    let total = wanted.len() as i64;
    progress(|p| p.images_total = total);

    // Download each one: the full size first, else the size a post used.
    let mut copies: HashMap<String, Result<String, &'static str>> = HashMap::new();
    for (done, full) in wanted.iter().enumerate() {
        let mut tries = vec![full.clone()];
        let used = addresses.get(full).map(Vec::as_slice).unwrap_or_default();
        tries.extend(used.iter().filter(|a| *a != full).cloned());
        let mut result = Err(images::UNREACHABLE);
        for address in tries {
            result = copy_image(state, &address).await;
            if result != Err(images::NOT_FOUND) {
                break;
            }
        }
        if result.is_ok() {
            report.images += 1;
        }
        copies.insert(full.clone(), result);
        progress(|p| p.images_done = done as i64 + 1);
    }

    // Point each post at the copies, and note the ones that stayed behind.
    for post in imported.iter_mut() {
        let mut replace: Vec<(String, String)> = Vec::new();
        let mut cover = None;
        let cover_url = post.item.cover_id.and_then(|id| covers.get(&id));
        for url in images_of(post.item, covers) {
            let full = images::full_size(&url);
            match copies.get(&full) {
                Some(Ok(copy)) => {
                    if cover_url == Some(&url) {
                        cover = Some(copy.clone());
                    }
                    replace.push((url, copy.clone()));
                }
                Some(Err(reason)) => {
                    if cover_url == Some(&url) {
                        cover = Some(url.clone());
                    }
                    let address = without_scheme(&full);
                    if !report.failed_images.iter().any(|f| f.address == address) {
                        report.failed_images.push(FailedImage {
                            post: post.item.title.clone(),
                            address,
                            reason,
                        });
                    }
                }
                None => {}
            }
        }
        post.content = images::point_to_copies(&post.content, &replace);
        let _ = sqlx::query("UPDATE posts SET content = ?, cover_image = ? WHERE id = ?")
            .bind(&post.content)
            .bind(cover)
            .bind(post.id)
            .execute(&state.pool)
            .await;
    }
}

/// Download one image and add it to Media. Gives back its new address.
async fn copy_image(state: &AppState, url: &str) -> Result<String, &'static str> {
    let max = crate::handlers::admin::media::MAX_UPLOAD_BYTES as u64;
    let address = url.to_string();
    let bytes = tokio::task::spawn_blocking(move || images::download(&address, max))
        .await
        .map_err(|_| images::UNREACHABLE)??;
    let name = url
        .split(['?', '#'])
        .next()
        .and_then(|path| path.rsplit('/').next())
        .unwrap_or("image");
    let filename = crate::handlers::admin::media::add_image(state, &url_decode(name), bytes)
        .await
        .map_err(|_| images::NOT_IMAGE)?;
    Ok(format!("/uploads/{filename}"))
}
