use sqlx::{Pool, Sqlite};
use std::collections::HashMap;

use crate::models::Tag;

pub fn slugify(name: &str) -> String {
    let mut slug = String::new();
    let mut last_was_dash = false;

    for c in name.to_lowercase().chars() {
        if c.is_ascii_alphanumeric() {
            slug.push(c);
            last_was_dash = false;
        } else if !last_was_dash {
            slug.push('-');
            last_was_dash = true;
        }
    }

    let trimmed = slug.trim_matches('-').to_string();
    if trimmed.is_empty() {
        "tag".to_string()
    } else {
        trimmed
    }
}

pub async fn get_all_tags(pool: &Pool<Sqlite>) -> Vec<Tag> {
    sqlx::query_as::<_, Tag>("SELECT id, name, slug FROM tags ORDER BY name ASC")
        .fetch_all(pool)
        .await
        .unwrap_or_default()
}

pub async fn get_tags_for_post(pool: &Pool<Sqlite>, post_id: i64) -> Vec<Tag> {
    sqlx::query_as::<_, Tag>(
        "SELECT t.id, t.name, t.slug FROM tags t
         INNER JOIN post_tags pt ON pt.tag_id = t.id
         WHERE pt.post_id = ?
         ORDER BY t.name ASC",
    )
    .bind(post_id)
    .fetch_all(pool)
    .await
    .unwrap_or_default()
}

pub async fn get_tags_for_posts(pool: &Pool<Sqlite>, post_ids: &[i64]) -> HashMap<i64, Vec<Tag>> {
    if post_ids.is_empty() {
        return HashMap::new();
    }

    let placeholders = post_ids.iter().map(|_| "?").collect::<Vec<_>>().join(",");
    let query = format!(
        "SELECT pt.post_id, t.id, t.name, t.slug FROM post_tags pt
         INNER JOIN tags t ON t.id = pt.tag_id
         WHERE pt.post_id IN ({})
         ORDER BY t.name ASC",
        placeholders
    );

    let mut q = sqlx::query_as::<_, (i64, i64, String, String)>(&query);
    for id in post_ids {
        q = q.bind(id);
    }

    let rows = q.fetch_all(pool).await.unwrap_or_default();

    let mut map: HashMap<i64, Vec<Tag>> = HashMap::new();
    for (post_id, tag_id, name, slug) in rows {
        map.entry(post_id).or_default().push(Tag {
            id: tag_id,
            name,
            slug,
        });
    }
    map
}

pub async fn set_post_tags(pool: &Pool<Sqlite>, post_id: i64, tag_ids: &[i64]) {
    let _ = sqlx::query("DELETE FROM post_tags WHERE post_id = ?")
        .bind(post_id)
        .execute(pool)
        .await;

    for tag_id in tag_ids {
        let _ = sqlx::query("INSERT OR IGNORE INTO post_tags (post_id, tag_id) VALUES (?, ?)")
            .bind(post_id)
            .bind(tag_id)
            .execute(pool)
            .await;
    }
}
