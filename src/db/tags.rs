//! Tags: listing them and linking them to posts.

use crate::app::models::Tag;

use sqlx::SqlitePool;
use std::collections::HashMap;

/// URL slug for a tag name.
pub fn slugify(name: &str) -> String {
    crate::content::text::slugify_or(name, "tag")
}

/// Every tag, by name.
pub async fn all(pool: &SqlitePool) -> Vec<Tag> {
    sqlx::query_as::<_, Tag>("SELECT id, name, slug FROM tags ORDER BY name ASC")
        .fetch_all(pool)
        .await
        .unwrap_or_default()
}

/// The id of the tag with this name (compared by slug, so "Rust" and "rust"
/// are the same tag).
pub async fn find_id(pool: &SqlitePool, name: &str) -> Option<i64> {
    crate::db::or_log(
        sqlx::query_scalar("SELECT id FROM tags WHERE slug = ?")
            .bind(slugify(name))
            .fetch_optional(pool)
            .await,
        "find tag",
    )
}

/// Create a tag and return its id.
pub async fn create(pool: &SqlitePool, name: &str) -> Result<i64, sqlx::Error> {
    let result = sqlx::query("INSERT INTO tags (name, slug) VALUES (?, ?)")
        .bind(name.trim())
        .bind(slugify(name))
        .execute(pool)
        .await?;
    Ok(result.last_insert_rowid())
}

pub async fn find_by_slug(pool: &SqlitePool, slug: &str) -> Option<Tag> {
    crate::db::or_log(
        sqlx::query_as::<_, Tag>("SELECT id, name, slug FROM tags WHERE slug = ?")
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
