//! Redirects the owner adds in Settings → Redirects (old address → new one).
//! Bloogla's own redirects for changed post addresses are in `slug_redirects`
//! (see `db/posts.rs`).

use crate::db::or_log;

use sqlx::SqlitePool;

/// A redirect in the list.
#[derive(sqlx::FromRow)]
pub struct Redirect {
    pub id: i64,
    /// The old address, like `/old-page` (decoded: `/hakkımızda`).
    pub from_path: String,
    /// Where it leads: an address on this site (`/about`) or another site's.
    pub to_address: String,
}

/// Every redirect, newest first.
pub async fn all(pool: &SqlitePool) -> Vec<Redirect> {
    or_log(
        sqlx::query_as("SELECT id, from_path, to_address FROM redirects ORDER BY id DESC")
            .fetch_all(pool)
            .await,
        "list redirects",
    )
}

/// Where the old address `from_path` leads, if it's in the list.
pub async fn target(pool: &SqlitePool, from_path: &str) -> Option<String> {
    or_log(
        sqlx::query_scalar("SELECT to_address FROM redirects WHERE from_path = ?")
            .bind(from_path)
            .fetch_optional(pool)
            .await,
        "find redirect",
    )
}

/// Add a redirect and return its id, or `None` when the old address already
/// leads somewhere.
pub async fn add(
    pool: &SqlitePool,
    from_path: &str,
    to_address: &str,
) -> Result<Option<i64>, sqlx::Error> {
    let result =
        sqlx::query("INSERT OR IGNORE INTO redirects (from_path, to_address) VALUES (?, ?)")
            .bind(from_path)
            .bind(to_address)
            .execute(pool)
            .await?;
    Ok((result.rows_affected() > 0).then(|| result.last_insert_rowid()))
}

/// Remove a redirect. `false` when there is no such redirect.
pub async fn delete(pool: &SqlitePool, id: i64) -> Result<bool, sqlx::Error> {
    let result = sqlx::query("DELETE FROM redirects WHERE id = ?")
        .bind(id)
        .execute(pool)
        .await?;
    Ok(result.rows_affected() > 0)
}
