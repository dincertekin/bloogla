//! Posts and pages: finding them, saving them (with revisions and redirects
//! for changed URLs) and deleting them.
//!
//! Pages are rows in the same `posts` table with `is_page = 1`.
//!
//! The admin editor saves through [`create`] and [`update`], which also hold
//! the rules about who may change what.

use crate::app::models::{CurrentUser, Post, PostForm};
use crate::content::text::slugify;
use crate::db::{or_log, tags};

use sqlx::SqlitePool;

/// Column list shared by every `Post` query. Keep in sync with the struct:
/// a missing column makes `FromRow` fail at runtime, not at compile time.
/// Cover image sizes come from the media library (for `/uploads/...` covers),
/// so themes can reserve the space before the image loads.
pub const POST_SELECT: &str = "SELECT id, title, slug, content, cover_image,
        (SELECT width FROM media WHERE media.filename = substr(posts.cover_image, 10)) AS cover_width,
        (SELECT height FROM media WHERE media.filename = substr(posts.cover_image, 10)) AS cover_height,
        (SELECT '/uploads/' || small_filename FROM media
         WHERE media.filename = substr(posts.cover_image, 10)) AS cover_small,
        (SELECT '/uploads/' || thumb_filename FROM media
         WHERE media.filename = substr(posts.cover_image, 10)) AS cover_thumb,
        COALESCE(views, 0) AS views, status, is_page, author_id,
        (SELECT NULLIF(name, '') FROM users WHERE users.id = posts.author_id) AS author_name,
        COALESCE(published_at, CURRENT_TIMESTAMP) AS published_at,
        COALESCE(created_at, CURRENT_TIMESTAMP) AS created_at, trashed_at
    FROM posts";

/// Posts visitors may see: published or scheduled, with a publish date in the
/// past, and not in the trash.
pub const PUBLIC_POST_FILTER: &str = "status IN ('published', 'scheduled') \
    AND datetime(published_at) <= datetime('now') AND trashed_at IS NULL";

/// Public posts that appear in listings and feeds (standalone pages are excluded).
pub const LISTED_POST_FILTER: &str = "status IN ('published', 'scheduled') \
    AND datetime(published_at) <= datetime('now') AND trashed_at IS NULL AND is_page = 0";

/// Revisions kept per post; older ones are deleted.
const MAX_REVISIONS: i64 = 25;

/// Slugs that would be hidden by built-in routes if used for a page.
const RESERVED_SLUGS: &[&str] = &[
    "admin",
    "api",
    // The list of posts when a page is the home page.
    "blog",
    "health",
    "post",
    "search",
    "setup",
    "static",
    "tag",
    "theme-assets",
    "uploads",
];

/// A post or page in any status.
pub async fn find(pool: &SqlitePool, id: i64) -> Option<Post> {
    or_log(
        sqlx::query_as::<_, Post>(&format!("{POST_SELECT} WHERE id = ?"))
            .bind(id)
            .fetch_optional(pool)
            .await,
        "find post",
    )
}

/// A post or page visitors may see.
pub async fn find_public(pool: &SqlitePool, slug: &str, is_page: bool) -> Option<Post> {
    or_log(
        sqlx::query_as::<_, Post>(&format!(
            "{POST_SELECT} WHERE slug = ? AND is_page = ? AND {PUBLIC_POST_FILTER}"
        ))
        .bind(slug)
        .bind(is_page)
        .fetch_optional(pool)
        .await,
        "find public post",
    )
}

/// The page chosen as the home page, while it's published.
pub async fn find_public_page(pool: &SqlitePool, id: i64) -> Option<Post> {
    or_log(
        sqlx::query_as::<_, Post>(&format!(
            "{POST_SELECT} WHERE id = ? AND is_page = 1 AND {PUBLIC_POST_FILTER}"
        ))
        .bind(id)
        .fetch_optional(pool)
        .await,
        "find home page",
    )
}

/// A link to another post, for "Previous post" and "Next post".
#[derive(Debug, serde::Serialize)]
pub struct PostLink {
    pub title: String,
    pub url: String,
}

/// The listed posts just before (older) and after (newer) `post`, by publish
/// date; the id breaks ties between posts published at the same moment.
pub async fn neighbours(pool: &SqlitePool, post: &Post) -> (Option<PostLink>, Option<PostLink>) {
    let find = |newer: bool| {
        let (compare, order) = if newer { (">", "ASC") } else { ("<", "DESC") };
        format!(
            "SELECT title, slug FROM posts WHERE {LISTED_POST_FILTER}
               AND (datetime(published_at) {compare} datetime(?)
                    OR (datetime(published_at) = datetime(?) AND id {compare} ?))
             ORDER BY datetime(published_at) {order}, id {order} LIMIT 1"
        )
    };
    let mut links = Vec::new();
    for newer in [false, true] {
        let row: Option<(String, String)> = or_log(
            sqlx::query_as(&find(newer))
                .bind(&post.published_at)
                .bind(&post.published_at)
                .bind(post.id)
                .fetch_optional(pool)
                .await,
            "find neighbouring post",
        );
        links.push(row.map(|(title, slug)| PostLink {
            title,
            url: crate::server::routes::post_path(&slug, false),
        }));
    }
    let next = links.pop().flatten();
    let previous = links.pop().flatten();
    (previous, next)
}

/// Pages visitors may see, for the menu editor in Settings:
/// in the order they were published (oldest first), so a new page goes last.
pub async fn public_pages(pool: &SqlitePool) -> Vec<Post> {
    or_log(
        sqlx::query_as::<_, Post>(&format!(
            "{POST_SELECT} WHERE is_page = 1 AND {PUBLIC_POST_FILTER}
             ORDER BY published_at, title COLLATE NOCASE"
        ))
        .fetch_all(pool)
        .await,
        "list public pages",
    )
}

/// Every post (or every page) in the admin lists, newest first, trash
/// included. People who may only edit their own get only theirs.
pub async fn for_admin_list(pool: &SqlitePool, me: &CurrentUser, is_page: bool) -> Vec<Post> {
    or_log(
        sqlx::query_as::<_, Post>(&format!(
            "{POST_SELECT} WHERE is_page = ? AND (? OR author_id = ?)
             ORDER BY published_at DESC, id DESC"
        ))
        .bind(is_page)
        .bind(me.can_edit_all())
        .bind(me.id)
        .fetch_all(pool)
        .await,
        "list posts",
    )
}

/// How many posts match `filter` (an SQL condition, with `binds` for its `?`s).
pub async fn count_where(pool: &SqlitePool, filter: &str, binds: &[&str]) -> i64 {
    let sql = format!("SELECT COUNT(*) FROM posts WHERE {filter}");
    let mut query = sqlx::query_scalar::<_, i64>(&sql);
    for bind in binds {
        query = query.bind(*bind);
    }
    or_log(query.fetch_one(pool).await, "count listing")
}

/// Posts matching `filter` (as in [`count_where`]), newest first: `limit`
/// of them, after skipping `offset`.
pub async fn list_where(
    pool: &SqlitePool,
    filter: &str,
    binds: &[&str],
    limit: i64,
    offset: i64,
) -> Vec<Post> {
    let sql = format!("{POST_SELECT} WHERE {filter} ORDER BY published_at DESC LIMIT ? OFFSET ?");
    let mut query = sqlx::query_as::<_, Post>(&sql);
    for bind in binds {
        query = query.bind(*bind);
    }
    or_log(
        query.bind(limit).bind(offset).fetch_all(pool).await,
        "fetch listing",
    )
}

/// The id of a public post (not a page) by its address.
pub async fn public_post_id(pool: &SqlitePool, slug: &str) -> Option<i64> {
    or_log(
        sqlx::query_scalar(&format!(
            "SELECT id FROM posts WHERE slug = ? AND is_page = 0 AND {PUBLIC_POST_FILTER}"
        ))
        .bind(slug)
        .fetch_optional(pool)
        .await,
        "find post",
    )
}

/// A post's title.
pub async fn title_of(pool: &SqlitePool, id: i64) -> String {
    or_log(
        sqlx::query_scalar("SELECT title FROM posts WHERE id = ?")
            .bind(id)
            .fetch_optional(pool)
            .await,
        "post title",
    )
    .unwrap_or_default()
}

/// A public post or page that holds a contact form: `(slug, is_page, title)`.
pub async fn with_contact_form(pool: &SqlitePool, id: i64) -> Option<(String, bool, String)> {
    or_log(
        sqlx::query_as(&format!(
            "SELECT slug, is_page, title FROM posts
             WHERE id = ? AND content LIKE '%[contact-form%' AND {PUBLIC_POST_FILTER}"
        ))
        .bind(id)
        .fetch_optional(pool)
        .await,
        "find the contact form's page",
    )
}

/// Whether a page (not in the trash) has this address.
pub async fn page_exists(pool: &SqlitePool, slug: &str) -> bool {
    let found: Option<i64> = or_log(
        sqlx::query_scalar(
            "SELECT id FROM posts WHERE slug = ? AND is_page = 1 AND trashed_at IS NULL",
        )
        .bind(slug)
        .fetch_optional(pool)
        .await,
        "find a page",
    );
    found.is_some()
}

/// Where an old address of a public post or page moved to:
/// `(current slug, is_page)`.
pub async fn moved_to(pool: &SqlitePool, old_slug: &str) -> Option<(String, bool)> {
    or_log(
        sqlx::query_as(&format!(
            "SELECT slug, is_page FROM posts
             WHERE id = (SELECT post_id FROM slug_redirects WHERE old_slug = ?)
               AND {PUBLIC_POST_FILTER}"
        ))
        .bind(old_slug)
        .fetch_optional(pool)
        .await,
        "slug redirect lookup",
    )
}

/// The newest listed posts, for the RSS feed.
pub async fn newest_listed(pool: &SqlitePool, limit: i64) -> Vec<Post> {
    or_log(
        sqlx::query_as::<_, Post>(&format!(
            "{POST_SELECT} WHERE {LISTED_POST_FILTER} ORDER BY published_at DESC LIMIT ?"
        ))
        .bind(limit)
        .fetch_all(pool)
        .await,
        "rss posts",
    )
}

/// Every listed post for the archive page, newest first:
/// `(title, slug, published_at)`. Only what the list shows is read.
pub async fn for_archive(pool: &SqlitePool) -> Vec<(String, String, String)> {
    or_log(
        sqlx::query_as(&format!(
            "SELECT title, slug, published_at FROM posts
             WHERE {LISTED_POST_FILTER} ORDER BY published_at DESC"
        ))
        .fetch_all(pool)
        .await,
        "archive posts",
    )
}

/// Every public post and page for the sitemap: `(id, slug, is_page, published_at)`.
pub async fn for_sitemap(pool: &SqlitePool) -> Vec<(i64, String, bool, String)> {
    or_log(
        sqlx::query_as(&format!(
            "SELECT id, slug, is_page, published_at FROM posts
             WHERE {PUBLIC_POST_FILTER} ORDER BY published_at DESC"
        ))
        .fetch_all(pool)
        .await,
        "sitemap posts",
    )
}

/// Whether the owner wrote a post of their own. Sample posts are added at
/// setup, so a post written more than a few minutes after the first account
/// was made is theirs.
pub async fn owner_wrote_a_post(pool: &SqlitePool) -> bool {
    or_log(
        sqlx::query_scalar(
            "SELECT EXISTS (SELECT 1 FROM posts WHERE is_page = 0
             AND created_at > (SELECT datetime(MIN(created_at), '+5 minutes') FROM users))",
        )
        .fetch_one(pool)
        .await,
        "checklist posts",
    )
}

/// The 10 newest saved versions of a post: `(revision id, saved at)`.
pub async fn revisions(pool: &SqlitePool, post_id: i64) -> Vec<(i64, String)> {
    or_log(
        sqlx::query_as(
            "SELECT id, saved_at FROM post_revisions WHERE post_id = ?
             ORDER BY id DESC LIMIT 10",
        )
        .bind(post_id)
        .fetch_all(pool)
        .await,
        "list revisions",
    )
}

/// One saved version of a post: `(title, content)`.
pub async fn revision(pool: &SqlitePool, post_id: i64, id: i64) -> Option<(String, String)> {
    or_log(
        sqlx::query_as("SELECT title, content FROM post_revisions WHERE id = ? AND post_id = ?")
            .bind(id)
            .bind(post_id)
            .fetch_optional(pool)
            .await,
        "load revision",
    )
}

// ---- Private preview links ----

/// The code in a post's private preview link, if it's shared.
pub async fn preview_token(pool: &SqlitePool, id: i64) -> Option<String> {
    or_log(
        sqlx::query_scalar("SELECT preview_token FROM posts WHERE id = ?")
            .bind(id)
            .fetch_optional(pool)
            .await,
        "load preview link",
    )
    .flatten()
}

/// Share a post with a private link (`Some(code)`), or stop (`None`).
pub async fn set_preview_token(
    pool: &SqlitePool,
    id: i64,
    token: Option<&str>,
) -> Result<(), sqlx::Error> {
    sqlx::query("UPDATE posts SET preview_token = ? WHERE id = ?")
        .bind(token)
        .bind(id)
        .execute(pool)
        .await?;
    Ok(())
}

/// The post a private preview link shows (not one in the trash).
pub async fn find_by_preview_token(pool: &SqlitePool, token: &str) -> Option<Post> {
    or_log(
        sqlx::query_as::<_, Post>(&format!(
            "{POST_SELECT} WHERE preview_token = ? AND trashed_at IS NULL"
        ))
        .bind(token)
        .fetch_optional(pool)
        .await,
        "find previewed post",
    )
}

// ---- Import and export ----

/// A post or page as it's exported.
#[derive(sqlx::FromRow)]
pub struct ExportedPost {
    pub id: i64,
    pub title: String,
    pub slug: String,
    pub content: String,
    pub cover_image: Option<String>,
    pub status: String,
    pub published_at: String,
    pub is_page: bool,
    pub author_id: Option<i64>,
}

/// Every post and page outside the trash, drafts too, oldest first.
pub async fn all_for_export(pool: &SqlitePool) -> Vec<ExportedPost> {
    or_log(
        sqlx::query_as(
            "SELECT id, title, slug, content, cover_image, status, published_at, is_page,
                    author_id
             FROM posts WHERE trashed_at IS NULL ORDER BY published_at",
        )
        .fetch_all(pool)
        .await,
        "load posts for export",
    )
}

/// A post or page brought in from another blog, saved as it was there.
pub struct ImportedPost<'a> {
    pub title: &'a str,
    pub slug: &'a str,
    pub content: &'a str,
    pub cover_image: Option<&'a str>,
    pub status: &'a str,
    pub published_at: &'a str,
    pub is_page: bool,
    pub author_id: i64,
}

/// Save an imported post or page and return its id.
pub async fn add_imported(pool: &SqlitePool, post: &ImportedPost<'_>) -> Result<i64, sqlx::Error> {
    let result = sqlx::query(
        "INSERT INTO posts (title, slug, content, cover_image, status, published_at, is_page,
                            author_id)
         VALUES (?, ?, ?, ?, ?, ?, ?, ?)",
    )
    .bind(post.title)
    .bind(post.slug)
    .bind(post.content)
    .bind(post.cover_image)
    .bind(post.status)
    .bind(post.published_at)
    .bind(post.is_page)
    .bind(post.author_id)
    .execute(pool)
    .await?;
    Ok(result.last_insert_rowid())
}

/// Make `old_slug` lead to post `id` (unless it already leads somewhere).
pub async fn add_old_slug(pool: &SqlitePool, old_slug: &str, id: i64) {
    let _ = sqlx::query("INSERT OR IGNORE INTO slug_redirects (old_slug, post_id) VALUES (?, ?)")
        .bind(old_slug)
        .bind(id)
        .execute(pool)
        .await;
}

/// Replace a post's text and cover (after its images were copied).
pub async fn set_content_and_cover(pool: &SqlitePool, id: i64, content: &str, cover: Option<&str>) {
    let saved = sqlx::query("UPDATE posts SET content = ?, cover_image = ? WHERE id = ?")
        .bind(content)
        .bind(cover)
        .bind(id)
        .execute(pool)
        .await;
    if let Err(e) = saved {
        tracing::error!("Failed to save post {id}: {e}");
    }
}

/// Fill in `tags` for several posts with one query.
pub async fn load_tags(pool: &SqlitePool, posts: &mut [Post]) {
    let ids: Vec<i64> = posts.iter().map(|p| p.id).collect();
    let mut tag_map = tags::for_posts(pool, &ids).await;
    for post in posts {
        post.tags = tag_map.remove(&post.id).unwrap_or_default();
    }
}

/// True when a post is visible to visitors right now.
pub fn is_live(post: &Post) -> bool {
    let now = chrono::Utc::now().format("%Y-%m-%d %H:%M:%S").to_string();
    post.status != "draft"
        && post.trashed_at.is_none()
        && post.published_at.replace('T', " ") <= now
}

/// Why saving a post failed.
#[derive(Debug, PartialEq)]
pub enum SaveError {
    NotFound,
    Forbidden,
    MissingTitle,
    Database,
}

fn database_error(context: &str, e: sqlx::Error) -> SaveError {
    tracing::error!("Database error ({context}): {e}");
    SaveError::Database
}

/// Author and kind (`is_page`) of a post, if it exists.
pub async fn owner(pool: &SqlitePool, id: i64) -> Option<(Option<i64>, bool)> {
    or_log(
        sqlx::query_as("SELECT author_id, is_page FROM posts WHERE id = ?")
            .bind(id)
            .fetch_optional(pool)
            .await,
        "load post owner",
    )
}

/// Whether `me` may change the post with `id` (false if it doesn't exist).
pub async fn may_edit(pool: &SqlitePool, me: &CurrentUser, id: i64) -> bool {
    owner(pool, id)
        .await
        .is_some_and(|(author_id, is_page)| me.can_edit(author_id, is_page))
}

/// Create a post or page written by `me`. Returns the new id.
pub async fn create(
    pool: &SqlitePool,
    me: &CurrentUser,
    is_page: bool,
    form: &PostForm,
) -> Result<i64, SaveError> {
    if is_page && !me.can_edit_all() {
        return Err(SaveError::Forbidden);
    }
    let title = form.title.trim();
    if title.is_empty() {
        return Err(SaveError::MissingTitle);
    }

    let slug_source = match form.slug.as_deref().map(str::trim) {
        Some(custom) if !custom.is_empty() => custom,
        _ => title,
    };
    let slug = unique_slug(pool, slug_source, None)
        .await
        .map_err(|e| database_error("generate slug", e))?;

    let tz = crate::db::settings::load(pool).await.timezone;
    let post = form.to_post(0, is_page, Some(me.id), slug, tz);
    let id = sqlx::query(
        "INSERT INTO posts (title, slug, content, cover_image, status, published_at, is_page,
                            author_id)
         VALUES (?, ?, ?, ?, ?, ?, ?, ?)",
    )
    .bind(title)
    .bind(&post.slug)
    .bind(&post.content)
    .bind(&post.cover_image)
    .bind(&post.status)
    .bind(&post.published_at)
    .bind(post.is_page)
    .bind(me.id)
    .execute(pool)
    .await
    .map_err(|e| database_error("insert post", e))?
    .last_insert_rowid();

    let tag_ids = tags::with_new(pool, &form.tag_ids, &form.new_tags).await;
    tags::set_for_post(pool, id, &tag_ids).await;
    Ok(id)
}

/// Save changes to a post `me` may edit, keeping the previous version as a
/// revision and redirecting the old address if the slug changed.
pub async fn update(
    pool: &SqlitePool,
    me: &CurrentUser,
    id: i64,
    form: &PostForm,
) -> Result<(), SaveError> {
    let existing: Option<(String, bool, String, String, Option<i64>)> = or_log(
        sqlx::query_as("SELECT slug, is_page, title, content, author_id FROM posts WHERE id = ?")
            .bind(id)
            .fetch_optional(pool)
            .await,
        "load post",
    );
    let (old_slug, is_page, old_title, old_content, author_id) =
        existing.ok_or(SaveError::NotFound)?;
    if !me.can_edit(author_id, is_page) {
        return Err(SaveError::Forbidden);
    }
    let title = form.title.trim();
    if title.is_empty() {
        return Err(SaveError::MissingTitle);
    }

    // The URL only changes when the slug is edited, so links keep working.
    let slug = match form.slug.as_deref().map(str::trim) {
        Some(custom) if !custom.is_empty() && custom != old_slug => {
            unique_slug(pool, custom, Some(id))
                .await
                .map_err(|e| database_error("generate slug", e))?
        }
        _ => old_slug.clone(),
    };

    let tz = crate::db::settings::load(pool).await.timezone;
    let post = form.to_post(id, is_page, author_id, slug, tz);
    if old_title != title || old_content != post.content {
        save_revision(pool, id, &old_title, &old_content).await;
    }

    sqlx::query(
        "UPDATE posts SET title = ?, slug = ?, content = ?, cover_image = ?, status = ?,
                published_at = ?
         WHERE id = ?",
    )
    .bind(title)
    .bind(&post.slug)
    .bind(&post.content)
    .bind(&post.cover_image)
    .bind(&post.status)
    .bind(&post.published_at)
    .bind(id)
    .execute(pool)
    .await
    .map_err(|e| database_error("update post", e))?;

    if post.slug != old_slug {
        let _ = sqlx::query("DELETE FROM slug_redirects WHERE old_slug = ?")
            .bind(&post.slug)
            .execute(pool)
            .await;
        let _ = sqlx::query(
            "INSERT INTO slug_redirects (old_slug, post_id) VALUES (?, ?)
             ON CONFLICT(old_slug) DO UPDATE SET post_id = excluded.post_id",
        )
        .bind(&old_slug)
        .bind(id)
        .execute(pool)
        .await;
    }

    let tag_ids = tags::with_new(pool, &form.tag_ids, &form.new_tags).await;
    tags::set_for_post(pool, id, &tag_ids).await;
    Ok(())
}

// ---- The trash, and changing many posts at once ----
// Each of these takes a list of posts (one, or the ones ticked in the list)
// and changes only those `me` may edit; it gives back how many changed.

/// Of `ids`, the posts and pages `me` may change.
async fn editable(pool: &SqlitePool, me: &CurrentUser, ids: &[i64]) -> Vec<i64> {
    let mut allowed = Vec::new();
    for &id in ids {
        if may_edit(pool, me, id).await && !allowed.contains(&id) {
            allowed.push(id);
        }
    }
    allowed
}

/// Run `sql` (with one `?` for the id) for each post `me` may change.
async fn change_each(
    pool: &SqlitePool,
    me: &CurrentUser,
    ids: &[i64],
    sql: &str,
    context: &str,
) -> Result<u64, SaveError> {
    let allowed = editable(pool, me, ids).await;
    if allowed.is_empty() {
        // One post asked for: say whether it's missing or someone else's.
        return match ids {
            [id] if owner(pool, *id).await.is_some() => Err(SaveError::Forbidden),
            _ => Err(SaveError::NotFound),
        };
    }
    let mut changed = 0;
    for id in allowed {
        changed += sqlx::query(sql)
            .bind(id)
            .execute(pool)
            .await
            .map_err(|e| database_error(context, e))?
            .rows_affected();
    }
    Ok(changed)
}

/// Move posts to the trash: off the site, and back with [`restore`].
pub async fn move_to_trash(
    pool: &SqlitePool,
    me: &CurrentUser,
    ids: &[i64],
) -> Result<u64, SaveError> {
    let sql = "UPDATE posts SET trashed_at = CURRENT_TIMESTAMP WHERE id = ? AND trashed_at IS NULL";
    change_each(pool, me, ids, sql, "move to trash").await
}

/// Take posts out of the trash, as they were before.
pub async fn restore(pool: &SqlitePool, me: &CurrentUser, ids: &[i64]) -> Result<u64, SaveError> {
    let sql = "UPDATE posts SET trashed_at = NULL WHERE id = ? AND trashed_at IS NOT NULL";
    change_each(pool, me, ids, sql, "restore from trash").await
}

/// Delete posts in the trash for good (posts not in the trash stay).
pub async fn delete_forever(
    pool: &SqlitePool,
    me: &CurrentUser,
    ids: &[i64],
) -> Result<u64, SaveError> {
    let sql = "DELETE FROM posts WHERE id = ? AND trashed_at IS NOT NULL";
    change_each(pool, me, ids, sql, "delete post").await
}

/// Publish posts. A draft dated in the past gets today's date, so it shows
/// up as new instead of hidden among old posts; one dated in the future is
/// scheduled for then.
pub async fn publish(pool: &SqlitePool, me: &CurrentUser, ids: &[i64]) -> Result<u64, SaveError> {
    let sql = "UPDATE posts SET
           published_at = CASE WHEN status = 'draft' AND datetime(published_at) <= datetime('now')
                               THEN datetime('now') ELSE published_at END,
           status = CASE WHEN datetime(published_at) > datetime('now')
                         THEN 'scheduled' ELSE 'published' END
         WHERE id = ? AND trashed_at IS NULL";
    change_each(pool, me, ids, sql, "publish").await
}

/// Make posts drafts (off the site until published again).
pub async fn make_draft(
    pool: &SqlitePool,
    me: &CurrentUser,
    ids: &[i64],
) -> Result<u64, SaveError> {
    let sql = "UPDATE posts SET status = 'draft' WHERE id = ? AND trashed_at IS NULL";
    change_each(pool, me, ids, sql, "make draft").await
}

/// Delete everything in the posts' (or pages') trash that `me` may change.
pub async fn empty_trash(
    pool: &SqlitePool,
    me: &CurrentUser,
    is_page: bool,
) -> Result<u64, SaveError> {
    sqlx::query(
        "DELETE FROM posts WHERE trashed_at IS NOT NULL AND is_page = ? AND (? OR author_id = ?)",
    )
    .bind(is_page)
    .bind(me.can_edit_all())
    .bind(me.id)
    .execute(pool)
    .await
    .map(|done| done.rows_affected())
    .map_err(|e| database_error("empty trash", e))
}

/// How long posts stay in the trash before they're deleted by themselves.
pub const TRASH_DAYS: i64 = 30;

/// Delete posts that have been in the trash longer than [`TRASH_DAYS`]
/// (run by the hourly housekeeping).
pub async fn delete_old_trash(pool: &SqlitePool) {
    let deleted = sqlx::query("DELETE FROM posts WHERE trashed_at <= datetime('now', ?)")
        .bind(format!("-{TRASH_DAYS} days"))
        .execute(pool)
        .await;
    match deleted {
        Ok(done) if done.rows_affected() > 0 => {
            tracing::info!(
                "Deleted {} posts that were in the trash for {TRASH_DAYS} days",
                done.rows_affected()
            );
        }
        Ok(_) => {}
        Err(e) => tracing::error!("Couldn't empty old trash: {e}"),
    }
}

/// Keep the version of a post that is about to be overwritten.
async fn save_revision(pool: &SqlitePool, post_id: i64, title: &str, content: &str) {
    let saved =
        sqlx::query("INSERT INTO post_revisions (post_id, title, content) VALUES (?, ?, ?)")
            .bind(post_id)
            .bind(title)
            .bind(content)
            .execute(pool)
            .await;
    if let Err(e) = saved {
        tracing::error!("Failed to save revision: {e}");
        return;
    }
    let _ = sqlx::query(
        "DELETE FROM post_revisions WHERE post_id = ? AND id NOT IN
           (SELECT id FROM post_revisions WHERE post_id = ? ORDER BY id DESC LIMIT ?)",
    )
    .bind(post_id)
    .bind(post_id)
    .bind(MAX_REVISIONS)
    .execute(pool)
    .await;
}

/// A slug for `text` that no other post uses, adding `-2`, `-3`... if needed.
pub async fn unique_slug(
    pool: &SqlitePool,
    text: &str,
    exclude_id: Option<i64>,
) -> Result<String, sqlx::Error> {
    let base = slugify(text);
    let mut candidate = base.clone();
    let mut counter = 2;

    loop {
        let exists: Option<i64> =
            sqlx::query_scalar("SELECT id FROM posts WHERE slug = ? AND (? IS NULL OR id != ?)")
                .bind(&candidate)
                .bind(exclude_id)
                .bind(exclude_id)
                .fetch_optional(pool)
                .await?;

        if exists.is_none() && !RESERVED_SLUGS.contains(&candidate.as_str()) {
            return Ok(candidate);
        }
        candidate = format!("{base}-{counter}");
        counter += 1;
    }
}

/// Turn free text into a safe SQLite full-text (FTS5) query: every word must
/// appear, and each word also matches longer words starting with it
/// ("istan" → "istanbul").
///
/// Returns `None` when the text has no searchable words.
pub fn search_query(input: &str) -> Option<String> {
    let terms: Vec<String> = input
        .split(|c: char| !c.is_alphanumeric())
        .filter(|w| !w.is_empty())
        .take(12)
        .map(|w| format!("\"{}\"*", w.to_lowercase()))
        .collect();
    (!terms.is_empty()).then(|| terms.join(" "))
}

#[cfg(test)]
mod tests {
    use super::search_query;

    #[test]
    fn search_query_quotes_words_and_drops_syntax() {
        assert_eq!(search_query("Rust  web"), Some("\"rust\"* \"web\"*".into()));
        assert_eq!(
            search_query("a\" OR b*"),
            Some("\"a\"* \"or\"* \"b\"*".into())
        );
        assert_eq!(search_query("!!! ---"), None);
    }
}
