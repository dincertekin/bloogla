//! Messages that visitors send with the contact form.

use crate::db::or_log;

use sqlx::SqlitePool;

/// A message with the page its form is on (empty if that page is gone).
#[derive(sqlx::FromRow)]
pub struct Message {
    pub id: i64,
    pub name: String,
    pub email: String,
    pub message: String,
    pub created_at: String,
    /// `None` until someone opens Messages.
    pub read_at: Option<String>,
    pub title: Option<String>,
    pub slug: Option<String>,
    pub is_page: Option<bool>,
}

/// Every message, newest first.
pub async fn all(pool: &SqlitePool) -> Vec<Message> {
    or_log(
        sqlx::query_as(
            "SELECT m.id, m.name, m.email, m.message, m.created_at, m.read_at,
                    p.title, p.slug, p.is_page
             FROM messages m LEFT JOIN posts p ON p.id = m.post_id
             ORDER BY m.id DESC",
        )
        .fetch_all(pool)
        .await,
        "list messages",
    )
}

/// How many messages nobody has read yet (the number on the menu badge).
pub async fn count_unread(pool: &SqlitePool) -> i64 {
    or_log(
        sqlx::query_scalar("SELECT COUNT(*) FROM messages WHERE read_at IS NULL")
            .fetch_one(pool)
            .await,
        "count new messages",
    )
}

/// Mark every message as read.
pub async fn mark_all_read(pool: &SqlitePool) {
    or_log(
        sqlx::query("UPDATE messages SET read_at = CURRENT_TIMESTAMP WHERE read_at IS NULL")
            .execute(pool)
            .await
            .map(|_| ()),
        "mark messages read",
    );
}

/// Save a message sent from the form on page `page_id`.
pub async fn add(
    pool: &SqlitePool,
    page_id: i64,
    name: &str,
    email: &str,
    message: &str,
) -> Result<(), sqlx::Error> {
    sqlx::query("INSERT INTO messages (post_id, name, email, message) VALUES (?, ?, ?, ?)")
        .bind(page_id)
        .bind(name)
        .bind(email)
        .bind(message)
        .execute(pool)
        .await?;
    Ok(())
}

/// Delete a message. `false` when there is no such message.
pub async fn delete(pool: &SqlitePool, id: i64) -> Result<bool, sqlx::Error> {
    let result = sqlx::query("DELETE FROM messages WHERE id = ?")
        .bind(id)
        .execute(pool)
        .await?;
    Ok(result.rows_affected() > 0)
}
