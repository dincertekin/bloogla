//! Readers who get new posts by email: added by the form under posts,
//! confirmed with the link in the first email, removed by the unsubscribe
//! link or in Settings → Subscribers.

use crate::db::or_log;

use sqlx::SqlitePool;

/// A confirmed subscriber, for the list in Settings and for sending.
#[derive(sqlx::FromRow)]
pub struct Subscriber {
    pub id: i64,
    pub email: String,
    /// The secret in their unsubscribe link.
    pub token: String,
    pub confirmed_at: String,
}

/// What [`ask`] did with an address.
pub enum Asked {
    /// New or not confirmed yet: send the confirmation email with this token.
    Confirm(String),
    /// Already subscribed: nothing to send.
    AlreadyConfirmed,
}

/// Someone typed their address in the form. A new address waits for
/// confirmation; asking again keeps the same link.
pub async fn ask(pool: &SqlitePool, email: &str, new_token: &str) -> Result<Asked, sqlx::Error> {
    sqlx::query("INSERT OR IGNORE INTO subscribers (email, token) VALUES (?, ?)")
        .bind(email)
        .bind(new_token)
        .execute(pool)
        .await?;
    let (token, confirmed): (String, bool) =
        sqlx::query_as("SELECT token, confirmed_at IS NOT NULL FROM subscribers WHERE email = ?")
            .bind(email)
            .fetch_one(pool)
            .await?;
    Ok(if confirmed {
        Asked::AlreadyConfirmed
    } else {
        Asked::Confirm(token)
    })
}

/// The link in the first email was opened. Whether the token was found.
pub async fn confirm(pool: &SqlitePool, token: &str) -> bool {
    let result = sqlx::query(
        "UPDATE subscribers SET confirmed_at = COALESCE(confirmed_at, CURRENT_TIMESTAMP)
         WHERE token = ?",
    )
    .bind(token)
    .execute(pool)
    .await;
    or_log(result.map(|r| r.rows_affected() > 0), "confirm subscriber")
}

/// Whether `token` belongs to someone on the list (for the unsubscribe page).
pub async fn token_exists(pool: &SqlitePool, token: &str) -> bool {
    or_log(
        sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM subscribers WHERE token = ?)")
            .bind(token)
            .fetch_one(pool)
            .await,
        "find subscriber",
    )
}

/// Remove the subscriber with this unsubscribe token.
pub async fn unsubscribe(pool: &SqlitePool, token: &str) -> Result<bool, sqlx::Error> {
    let result = sqlx::query("DELETE FROM subscribers WHERE token = ?")
        .bind(token)
        .execute(pool)
        .await?;
    Ok(result.rows_affected() > 0)
}

/// Remove a subscriber from Settings → Subscribers.
pub async fn delete(pool: &SqlitePool, id: i64) -> Result<bool, sqlx::Error> {
    let result = sqlx::query("DELETE FROM subscribers WHERE id = ?")
        .bind(id)
        .execute(pool)
        .await?;
    Ok(result.rows_affected() > 0)
}

/// Everyone who confirmed, newest first.
pub async fn confirmed(pool: &SqlitePool) -> Vec<Subscriber> {
    or_log(
        sqlx::query_as(
            "SELECT id, email, token, confirmed_at FROM subscribers
             WHERE confirmed_at IS NOT NULL ORDER BY confirmed_at DESC, id DESC",
        )
        .fetch_all(pool)
        .await,
        "list subscribers",
    )
}

/// Addresses that never confirmed are forgotten after this many days.
const PENDING_DAYS: i64 = 7;

/// Forget addresses nobody confirmed (someone may have typed another
/// person's address). Called every hour.
pub async fn delete_unconfirmed(pool: &SqlitePool) {
    let result = sqlx::query(
        "DELETE FROM subscribers WHERE confirmed_at IS NULL
         AND datetime(created_at) < datetime('now', ?)",
    )
    .bind(format!("-{PENDING_DAYS} days"))
    .execute(pool)
    .await;
    if let Err(e) = result {
        tracing::error!("Could not remove unconfirmed subscribers: {e}");
    }
}
