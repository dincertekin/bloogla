//! Tags: listing them and linking them to posts.

use crate::app::models::Tag;

use sqlx::SqlitePool;
use std::collections::HashMap;

/// URL slug for a tag name.
pub fn slugify(name: &str) -> String {
    crate::content::text::slugify_or(name, "tag")
}

/// Every tag, by name (for the post editor; the site shows [`listed`]).
pub async fn all(pool: &SqlitePool) -> Vec<Tag> {
    sqlx::query_as::<_, Tag>("SELECT id, name, slug FROM tags ORDER BY name ASC")
        .fetch_all(pool)
        .await
        .unwrap_or_default()
}

/// Only tags used by a post visitors can see. Drafts and scheduled posts
/// that aren't out yet don't count, so their tags (often the name of
/// something not announced yet) stay private.
const HAS_LISTED_POST: &str = "EXISTS (SELECT 1 FROM post_tags pt
     JOIN posts ON posts.id = pt.post_id
     WHERE pt.tag_id = tags.id AND ";

/// Tags with at least one public post, by name: the ones the site shows.
pub async fn listed(pool: &SqlitePool) -> Vec<Tag> {
    let filter = crate::db::posts::LISTED_POST_FILTER;
    sqlx::query_as::<_, Tag>(&format!(
        "SELECT id, name, slug FROM tags WHERE {HAS_LISTED_POST}{filter}) ORDER BY name ASC"
    ))
    .fetch_all(pool)
    .await
    .unwrap_or_default()
}

/// A tag with at least one public post, for its page on the site.
pub async fn find_listed_by_slug(pool: &SqlitePool, slug: &str) -> Option<Tag> {
    let filter = crate::db::posts::LISTED_POST_FILTER;
    crate::db::or_log(
        sqlx::query_as::<_, Tag>(&format!(
            "SELECT id, name, slug FROM tags WHERE slug = ? AND {HAS_LISTED_POST}{filter})"
        ))
        .bind(slug)
        .fetch_optional(pool)
        .await,
        "find tag",
    )
}

/// Tags of several posts, keyed by post id.
pub async fn for_posts(pool: &SqlitePool, post_ids: &[i64]) -> HashMap<i64, Vec<Tag>> {
    if post_ids.is_empty() {
        return HashMap::new();
    }

    let placeholders = vec!["?"; post_ids.len()].join(",");
    let sql = format!(
        "SELECT pt.post_id, t.id, t.name, t.slug FROM post_tags pt
         INNER JOIN tags t ON t.id = pt.tag_id
         WHERE pt.post_id IN ({placeholders})
         ORDER BY t.name ASC"
    );
    let mut query = sqlx::query_as::<_, (i64, i64, String, String)>(&sql);
    for id in post_ids {
        query = query.bind(id);
    }

    let mut map: HashMap<i64, Vec<Tag>> = HashMap::new();
    for (post_id, id, name, slug) in query.fetch_all(pool).await.unwrap_or_default() {
        map.entry(post_id).or_default().push(Tag { id, name, slug });
    }
    map
}

/// Longest tag name accepted, and the most new tags one save may create.
const MAX_NAME_CHARS: usize = 50;
const MAX_NEW_TAGS: usize = 20;

/// `tag_ids` plus the tags named in `new_names` (typed in the editor; each
/// entry may hold several names separated by commas). Tags that don't exist
/// yet are created; a name that matches an existing tag ("rust" and "Rust")
/// reuses it.
pub async fn with_new(pool: &SqlitePool, tag_ids: &[i64], new_names: &[String]) -> Vec<i64> {
    let mut ids = tag_ids.to_vec();
    let names = new_names
        .iter()
        .flat_map(|entry| entry.split(','))
        .map(|name| name.split_whitespace().collect::<Vec<_>>().join(" "))
        .filter(|name| !name.is_empty())
        .map(|name| name.chars().take(MAX_NAME_CHARS).collect::<String>())
        .take(MAX_NEW_TAGS);
    for name in names {
        let slug = slugify(&name);
        let _ = sqlx::query("INSERT OR IGNORE INTO tags (name, slug) VALUES (?, ?)")
            .bind(&name)
            .bind(&slug)
            .execute(pool)
            .await;
        let id: Option<i64> = crate::db::or_log(
            sqlx::query_scalar("SELECT id FROM tags WHERE slug = ? OR name = ?")
                .bind(&slug)
                .bind(&name)
                .fetch_optional(pool)
                .await,
            "find new tag",
        );
        if let Some(id) = id.filter(|id| !ids.contains(id)) {
            ids.push(id);
        }
    }
    ids
}

/// Replace the tags of a post.
pub async fn set_for_post(pool: &SqlitePool, post_id: i64, tag_ids: &[i64]) {
    let result = async {
        let mut tx = pool.begin().await?;
        sqlx::query("DELETE FROM post_tags WHERE post_id = ?")
            .bind(post_id)
            .execute(&mut *tx)
            .await?;
        for tag_id in tag_ids {
            sqlx::query("INSERT OR IGNORE INTO post_tags (post_id, tag_id) VALUES (?, ?)")
                .bind(post_id)
                .bind(tag_id)
                .execute(&mut *tx)
                .await?;
        }
        tx.commit().await
    }
    .await;
    if let Err(e) = result {
        tracing::error!("Failed to save tags of post {post_id}: {e}");
    }
}
