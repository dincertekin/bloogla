//! Taking posts and pages out (Settings → Import & export → Export), for
//! moving to another blog tool. Drafts are included. (To keep a copy of the
//! whole site, people use Backup instead.)
//!
//! - [`wordpress_file`]: a WordPress export file, which WordPress and most
//!   blog tools can import. Posts are in HTML, as visitors see them, and
//!   images keep full addresses on this site, so WordPress can copy them.
//! - [`write_markdown_archive`]: one Markdown file per post and page, with
//!   the images, in a .zip.

use crate::app::state::AppState;
use crate::content::text::escape_html;
use crate::db::{or_log, tags};
use chrono::{NaiveDateTime, TimeZone};
use std::collections::HashMap;
use std::path::Path;

/// A post or page as it's exported.
#[derive(sqlx::FromRow)]
struct Exported {
    id: i64,
    title: String,
    slug: String,
    content: String,
    cover_image: Option<String>,
    status: String,
    published_at: String,
    is_page: bool,
    author_id: Option<i64>,
}

/// A comment as it's exported.
#[derive(sqlx::FromRow)]
struct ExportedComment {
    post_id: i64,
    id: i64,
    author_name: String,
    author_email: String,
    content: String,
    created_at: String,
}

async fn all_posts(state: &AppState) -> Vec<Exported> {
    or_log(
        sqlx::query_as(
            "SELECT id, title, slug, content, cover_image, status, published_at, is_page,
                    author_id
             FROM posts WHERE trashed_at IS NULL ORDER BY published_at",
        )
        .fetch_all(&state.pool)
        .await,
        "load posts for export",
    )
}

/// Images in Media: `(id, file name, original name)`.
async fn all_images(state: &AppState) -> Vec<(i64, String, String)> {
    or_log(
        sqlx::query_as("SELECT id, filename, original_name FROM media ORDER BY id")
            .fetch_all(&state.pool)
            .await,
        "load media for export",
    )
}

/// The WordPress export file for the whole site.
pub async fn wordpress_file(state: &AppState) -> String {
    let site = crate::db::settings::load(&state.pool).await;
    let base = state.config.base_url.trim_end_matches('/').to_string();
    let posts = all_posts(state).await;
    let images = all_images(state).await;
    let people: Vec<(i64, String, String)> = or_log(
        sqlx::query_as("SELECT id, email, name FROM users ORDER BY id")
            .fetch_all(&state.pool)
            .await,
        "load people for export",
    );
    let post_ids: Vec<i64> = posts.iter().map(|p| p.id).collect();
    let tags_by_post = tags::for_posts(&state.pool, &post_ids).await;
    let comments: Vec<ExportedComment> = or_log(
        sqlx::query_as(
            "SELECT post_id, id, author_name, author_email, content, created_at
             FROM comments WHERE status = 'approved' ORDER BY created_at",
        )
        .fetch_all(&state.pool)
        .await,
        "load comments for export",
    );
    let logins: HashMap<i64, &str> = people
        .iter()
        .map(|(id, email, _)| (*id, email.as_str()))
        .collect();
    // Images get ids after the posts', so the two never clash.
    let first_image_id = post_ids.iter().max().copied().unwrap_or(0) + 1;
    let image_ids: HashMap<String, i64> = images
        .iter()
        .map(|(id, filename, _)| (format!("/uploads/{filename}"), first_image_id + id))
        .collect();

    let mut xml = String::new();
    xml.push_str(r#"<?xml version="1.0" encoding="UTF-8" ?>
<rss version="2.0" xmlns:excerpt="http://wordpress.org/export/1.2/excerpt/" xmlns:content="http://purl.org/rss/1.0/modules/content/" xmlns:wfw="http://wellformedweb.org/CommentAPI/" xmlns:dc="http://purl.org/dc/elements/1.1/" xmlns:wp="http://wordpress.org/export/1.2/">
<channel>
"#);
    xml.push_str(&format!(
        "<title>{}</title>\n<link>{}</link>\n<description>{}</description>\n<language>{}</language>\n",
        escape_html(&site.blog_name),
        escape_html(&base),
        escape_html(&site.blog_description),
        site.language.code(),
    ));
    xml.push_str(&format!(
        "<wp:wxr_version>1.2</wp:wxr_version>\n<wp:base_site_url>{0}</wp:base_site_url>\n<wp:base_blog_url>{0}</wp:base_blog_url>\n",
        escape_html(&base)
    ));
    for (id, email, name) in &people {
        xml.push_str(&format!(
            "<wp:author><wp:author_id>{id}</wp:author_id><wp:author_login>{}</wp:author_login><wp:author_email>{}</wp:author_email><wp:author_display_name>{}</wp:author_display_name></wp:author>\n",
            cdata(email),
            cdata(email),
            cdata(name),
        ));
    }

    for post in &posts {
        let path = crate::server::routes::post_path(&post.slug, post.is_page);
        let html = crate::content::render(state, &post.content).await;
        let (date_utc, date_local, date_rss) = dates(&post.published_at, site.timezone);
        let status = match post.status.as_str() {
            "draft" => "draft",
            "scheduled" => "future",
            _ => "publish",
        };
        let login = post
            .author_id
            .and_then(|id| logins.get(&id))
            .copied()
            .unwrap_or_default();
        xml.push_str("<item>\n");
        xml.push_str(&format!(
            "<title>{}</title>\n<link>{}</link>\n<pubDate>{date_rss}</pubDate>\n<dc:creator>{}</dc:creator>\n<guid isPermaLink=\"false\">{}/?p={}</guid>\n",
            escape_html(&post.title),
            escape_html(&format!("{base}{path}")),
            cdata(login),
            escape_html(&base),
            post.id,
        ));
        xml.push_str(&format!(
            "<content:encoded>{}</content:encoded>\n<excerpt:encoded><![CDATA[]]></excerpt:encoded>\n",
            cdata(&with_full_addresses(&html, &base))
        ));
        xml.push_str(&format!(
            "<wp:post_id>{}</wp:post_id>\n<wp:post_date>{date_local}</wp:post_date>\n<wp:post_date_gmt>{date_utc}</wp:post_date_gmt>\n<wp:comment_status>open</wp:comment_status>\n<wp:ping_status>closed</wp:ping_status>\n<wp:post_name>{}</wp:post_name>\n<wp:status>{status}</wp:status>\n<wp:post_parent>0</wp:post_parent>\n<wp:menu_order>0</wp:menu_order>\n<wp:post_type>{}</wp:post_type>\n<wp:post_password></wp:post_password>\n<wp:is_sticky>0</wp:is_sticky>\n",
            post.id,
            cdata(&post.slug),
            if post.is_page { "page" } else { "post" },
        ));
        for tag in tags_by_post.get(&post.id).into_iter().flatten() {
            xml.push_str(&format!(
                "<category domain=\"post_tag\" nicename=\"{}\">{}</category>\n",
                escape_html(&tag.slug),
                cdata(&tag.name)
            ));
        }
        if let Some(image_id) = post.cover_image.as_ref().and_then(|c| image_ids.get(c)) {
            xml.push_str(&format!(
                "<wp:postmeta><wp:meta_key>_thumbnail_id</wp:meta_key><wp:meta_value>{image_id}</wp:meta_value></wp:postmeta>\n"
            ));
        }
        for comment in comments.iter().filter(|c| c.post_id == post.id) {
            let (date_utc, date_local, _) = dates(&comment.created_at, site.timezone);
            xml.push_str(&format!(
                "<wp:comment><wp:comment_id>{}</wp:comment_id><wp:comment_author>{}</wp:comment_author><wp:comment_author_email>{}</wp:comment_author_email><wp:comment_date>{date_local}</wp:comment_date><wp:comment_date_gmt>{date_utc}</wp:comment_date_gmt><wp:comment_content>{}</wp:comment_content><wp:comment_approved>1</wp:comment_approved><wp:comment_type>comment</wp:comment_type><wp:comment_parent>0</wp:comment_parent></wp:comment>\n",
                comment.id,
                cdata(&comment.author_name),
                cdata(&comment.author_email),
                cdata(&comment.content),
            ));
        }
        xml.push_str("</item>\n");
    }

    // The images, as WordPress's "attachments": its importer downloads these.
    for (id, filename, original_name) in &images {
        let url = format!("{base}/uploads/{filename}");
        xml.push_str(&format!(
            "<item>\n<title>{}</title>\n<wp:post_id>{}</wp:post_id>\n<wp:post_name>{}</wp:post_name>\n<wp:status>inherit</wp:status>\n<wp:post_type>attachment</wp:post_type>\n<wp:attachment_url>{}</wp:attachment_url>\n</item>\n",
            cdata(original_name),
            first_image_id + id,
            cdata(&crate::content::text::slugify(original_name)),
            cdata(&url),
        ));
    }
    xml.push_str("</channel>\n</rss>\n");
    xml
}

/// Text inside `<![CDATA[...]]>`, which keeps HTML as it is. The one thing
/// it can't hold, `]]>`, is split across two sections.
fn cdata(text: &str) -> String {
    format!("<![CDATA[{}]]>", text.replace("]]>", "]]]]><![CDATA[>"))
}

/// HTML with this site's image addresses written in full
/// (`/uploads/a.jpg` → `https://example.com/uploads/a.jpg`), so they still
/// work from another site.
fn with_full_addresses(html: &str, base: &str) -> String {
    html.replace("\"/uploads/", &format!("\"{base}/uploads/"))
        .replace(", /uploads/", &format!(", {base}/uploads/"))
}

/// A stored date (UTC) as WordPress writes it: in UTC, in the site's time
/// zone, and for RSS.
fn dates(utc: &str, tz: chrono_tz::Tz) -> (String, String, String) {
    let parsed = NaiveDateTime::parse_from_str(utc, "%Y-%m-%d %H:%M:%S")
        .unwrap_or_else(|_| chrono::Utc::now().naive_utc());
    let local = tz.from_utc_datetime(&parsed);
    (
        parsed.format("%Y-%m-%d %H:%M:%S").to_string(),
        local.format("%Y-%m-%d %H:%M:%S").to_string(),
        chrono::Utc.from_utc_datetime(&parsed).to_rfc2822(),
    )
}

/// The front matter at the top of a Markdown file: the title, date, tags and
/// so on, in the form Hugo, Jekyll and most Markdown tools read. Values are
/// written as JSON strings, which those tools read too, so quotes and
/// colons in a title are safe.
fn front_matter(post: &Exported, tag_names: &[String]) -> String {
    let quote = |text: &str| serde_json::to_string(text).unwrap_or_else(|_| "\"\"".into());
    let date = NaiveDateTime::parse_from_str(&post.published_at, "%Y-%m-%d %H:%M:%S")
        .map(|d| d.format("%Y-%m-%dT%H:%M:%SZ").to_string())
        .unwrap_or_default();
    let mut out = format!("---\ntitle: {}\ndate: {date}\n", quote(&post.title));
    if post.status == "draft" {
        out.push_str("draft: true\n");
    }
    if !tag_names.is_empty() {
        let list: Vec<String> = tag_names.iter().map(|t| quote(t)).collect();
        out.push_str(&format!("tags: [{}]\n", list.join(", ")));
    }
    if let Some(cover) = &post.cover_image {
        out.push_str(&format!("cover: {}\n", quote(cover)));
    }
    out.push_str("---\n\n");
    out
}

/// Write the Markdown .zip at `archive`: `posts/<address>.md`,
/// `pages/<address>.md` and the images in `uploads/`, so `/uploads/...`
/// addresses in the posts keep working when the folder is served as is.
pub async fn write_markdown_archive(state: &AppState, archive: &Path) -> Result<(), String> {
    let posts = all_posts(state).await;
    let post_ids: Vec<i64> = posts.iter().map(|p| p.id).collect();
    let tags_by_post = tags::for_posts(&state.pool, &post_ids).await;
    let files: Vec<(String, String)> = posts
        .iter()
        .map(|post| {
            let names: Vec<String> = tags_by_post
                .get(&post.id)
                .into_iter()
                .flatten()
                .map(|t| t.name.clone())
                .collect();
            let folder = if post.is_page { "pages" } else { "posts" };
            let text = format!("{}{}\n", front_matter(post, &names), post.content.trim());
            (format!("{folder}/{}.md", post.slug), text)
        })
        .collect();
    let uploads = state
        .config
        .folder
        .join(crate::handlers::admin::media::UPLOAD_DIR);
    let images: Vec<String> = all_images(state)
        .await
        .into_iter()
        .map(|(_, f, _)| f)
        .collect();
    let archive = archive.to_path_buf();
    tokio::task::spawn_blocking(move || write_zip(&archive, &files, &uploads, &images))
        .await
        .map_err(|e| e.to_string())?
}

fn write_zip(
    archive: &Path,
    files: &[(String, String)],
    uploads: &Path,
    images: &[String],
) -> Result<(), String> {
    use std::io::Write;
    use zip::write::SimpleFileOptions;

    let error = |e: &dyn std::fmt::Display| format!("could not write the export: {e}");
    let file = std::fs::File::create(archive).map_err(|e| error(&e))?;
    let mut zip = zip::ZipWriter::new(std::io::BufWriter::new(file));
    // Text is small and compresses well; images are already compressed.
    let text = SimpleFileOptions::default().compression_method(zip::CompressionMethod::Deflated);
    let stored = SimpleFileOptions::default().compression_method(zip::CompressionMethod::Stored);
    for (name, content) in files {
        zip.start_file(name.as_str(), text).map_err(|e| error(&e))?;
        zip.write_all(content.as_bytes()).map_err(|e| error(&e))?;
    }
    for name in images {
        let Ok(mut source) = std::fs::File::open(uploads.join(name)) else {
            continue; // a file deleted by hand: the rest still goes out
        };
        zip.start_file(format!("uploads/{name}"), stored)
            .map_err(|e| error(&e))?;
        std::io::copy(&mut source, &mut zip).map_err(|e| error(&e))?;
    }
    zip.finish()
        .map_err(|e| error(&e))?
        .flush()
        .map_err(|e| error(&e))
}
