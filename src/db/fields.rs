//! Custom fields: extra named values on a post, for themes and the API.

use sqlx::SqlitePool;
use std::collections::{BTreeMap, HashMap};

pub type Fields = BTreeMap<String, String>;

const MAX_FIELDS: usize = 50;
const MAX_KEY_LEN: usize = 40;
const MAX_VALUE_LEN: usize = 2000;

/// Turn a typed name into a key themes can use as `post.fields.<key>`:
/// lowercase letters, digits and underscores ("Reading Time" → "reading_time").
pub fn normalize_key(raw: &str) -> String {
    let slug = crate::content::text::slugify_or(raw, "");
    let key: String = slug
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() { c } else { '_' })
        .take(MAX_KEY_LEN)
        .collect();
    let key = key.trim_matches('_').to_string();
    // Keys starting with a digit can't be used with dot access in templates.
    if key.starts_with(|c: char| c.is_ascii_digit()) {
        format!("f_{key}")
    } else {
        key
    }
}

/// Clean submitted pairs: normalized keys, trimmed values, no empty keys, last one wins.
pub fn clean(pairs: impl IntoIterator<Item = (String, String)>) -> Fields {
    let mut fields = Fields::new();
    for (key, value) in pairs {
        let key = normalize_key(&key);
        if key.is_empty() || fields.len() >= MAX_FIELDS && !fields.contains_key(&key) {
            continue;
        }
        let value: String = value.trim().chars().take(MAX_VALUE_LEN).collect();
        fields.insert(key, value);
    }
    fields
}

/// Custom fields of several posts, keyed by post id.
pub async fn for_posts(pool: &SqlitePool, post_ids: &[i64]) -> HashMap<i64, Fields> {
    if post_ids.is_empty() {
        return HashMap::new();
    }
    let placeholders = vec!["?"; post_ids.len()].join(",");
    let sql =
        format!("SELECT post_id, key, value FROM post_fields WHERE post_id IN ({placeholders})");
    let mut query = sqlx::query_as::<_, (i64, String, String)>(&sql);
    for id in post_ids {
        query = query.bind(id);
    }
    let mut map: HashMap<i64, Fields> = HashMap::new();
    for (post_id, key, value) in query.fetch_all(pool).await.unwrap_or_default() {
        map.entry(post_id).or_default().insert(key, value);
    }
    map
}

/// Replace all fields of a post.
pub async fn set_for_post(pool: &SqlitePool, post_id: i64, fields: &Fields) {
    let result = async {
        let mut tx = pool.begin().await?;
        sqlx::query("DELETE FROM post_fields WHERE post_id = ?")
            .bind(post_id)
            .execute(&mut *tx)
            .await?;
        for (key, value) in fields {
            sqlx::query("INSERT INTO post_fields (post_id, key, value) VALUES (?, ?, ?)")
                .bind(post_id)
                .bind(key)
                .bind(value)
                .execute(&mut *tx)
                .await?;
        }
        tx.commit().await
    }
    .await;
    if let Err(e) = result {
        tracing::error!("Failed to save custom fields of post {post_id}: {e}");
    }
}

#[cfg(test)]
mod tests {
    use super::{clean, normalize_key};

    #[test]
    fn keys_work_in_templates() {
        assert_eq!(normalize_key("Reading Time"), "reading_time");
        assert_eq!(normalize_key("Konum (şehir)"), "konum_sehir");
        assert_eq!(normalize_key("2nd author"), "f_2nd_author");
        assert_eq!(normalize_key("!!!"), "");
    }

    #[test]
    fn clean_drops_empty_keys_and_trims() {
        let fields = clean(vec![
            ("Location".into(), "  İstanbul ".into()),
            ("".into(), "orphan".into()),
            ("location".into(), "Ankara".into()),
        ]);
        assert_eq!(fields.len(), 1);
        assert_eq!(fields["location"], "Ankara");
    }
}
