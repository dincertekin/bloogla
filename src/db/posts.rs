//! Posts and pages: finding them, saving them (with revisions and redirects
//! for changed URLs) and deleting them.
//!
//! Pages are rows in the same `posts` table with `is_page = 1`.
//!
//! The admin editor and the JSON API both save through [`create`] and
//! [`update`], so the rules about who may change what live in one place.

use crate::app::models::{CurrentUser, Post, PostForm};
use crate::content::text::slugify;
use crate::db::{fields, or_log, tags};

use sqlx::SqlitePool;

/// Column list shared by every `Post` query. Keep in sync with the struct:
/// a missing column makes `FromRow` fail at runtime, not at compile time.
/// Cover image sizes come from the media library (for `/uploads/...` covers),
/// so themes can reserve the space before the image loads.
pub const POST_SELECT: &str = "SELECT id, title, slug, content, cover_image,
        (SELECT width FROM media WHERE media.filename = substr(posts.cover_image, 10)) AS cover_width,
        (SELECT height FROM media WHERE media.filename = substr(posts.cover_image, 10)) AS cover_height,
        COALESCE(views, 0) AS views, status, is_page, author_id,
        (SELECT NULLIF(name, '') FROM users WHERE users.id = posts.author_id) AS author_name,
        COALESCE(published_at, CURRENT_TIMESTAMP) AS published_at,
        COALESCE(created_at, CURRENT_TIMESTAMP) AS created_at
    FROM posts";

/// Posts visitors may see: published or scheduled, with a publish date in the past.
pub const PUBLIC_POST_FILTER: &str =
    "status IN ('published', 'scheduled') AND datetime(published_at) <= datetime('now')";

/// Public posts that appear in listings and feeds (standalone pages are excluded).
pub const LISTED_POST_FILTER: &str = "status IN ('published', 'scheduled') \
    AND datetime(published_at) <= datetime('now') AND is_page = 0";

/// Revisions kept per post; older ones are deleted.
const MAX_REVISIONS: i64 = 25;

/// Slugs that would be hidden by built-in routes if used for a page.
const RESERVED_SLUGS: &[&str] = &[
    "admin",
    "api",
    "health",
    "post",
    "search",
    "setup",
    "static",
    "subscribe",
    "tag",
    "theme-assets",
    "unsubscribe",
    "uploads",
    "webmention",
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

/// Fill in `tags` and `fields` for several posts with two queries in total.
pub async fn load_tags_and_fields(pool: &SqlitePool, posts: &mut [Post]) {
    let ids: Vec<i64> = posts.iter().map(|p| p.id).collect();
    let mut tag_map = tags::for_posts(pool, &ids).await;
    let mut field_map = fields::for_posts(pool, &ids).await;
    for post in posts {
        post.tags = tag_map.remove(&post.id).unwrap_or_default();
        post.fields = field_map.remove(&post.id).unwrap_or_default();
    }
}

/// True when a post is visible to visitors right now.
pub fn is_live(post: &Post) -> bool {
    let now = chrono::Utc::now().format("%Y-%m-%d %H:%M:%S").to_string();
    post.status != "draft" && post.published_at.replace('T', " ") <= now
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

    let post = form.to_post(0, is_page, Some(me.id), slug);
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

    tags::set_for_post(pool, id, &form.tag_ids).await;
    fields::set_for_post(pool, id, &form.fields()).await;
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

    let post = form.to_post(id, is_page, author_id, slug);
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

    tags::set_for_post(pool, id, &form.tag_ids).await;
    fields::set_for_post(pool, id, &form.fields()).await;
    Ok(())
}

/// Delete a post `me` may edit.
pub async fn delete(pool: &SqlitePool, me: &CurrentUser, id: i64) -> Result<(), SaveError> {
    let (author_id, is_page) = owner(pool, id).await.ok_or(SaveError::NotFound)?;
    if !me.can_edit(author_id, is_page) {
        return Err(SaveError::Forbidden);
    }
    match sqlx::query("DELETE FROM posts WHERE id = ?")
        .bind(id)
        .execute(pool)
        .await
    {
        Ok(r) if r.rows_affected() > 0 => Ok(()),
        Ok(_) => Err(SaveError::NotFound),
        Err(e) => Err(database_error("delete post", e)),
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
