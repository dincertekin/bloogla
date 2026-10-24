//! Reader comments: saving them, the moderation queue and the approved ones
//! shown under each post.

use crate::db::or_log;

use sqlx::SqlitePool;
use std::collections::HashMap;

/// A comment as stored.
#[derive(sqlx::FromRow)]
pub struct Comment {
    pub id: i64,
    pub post_id: i64,
    pub author_name: String,
    pub author_email: String,
    pub content: String,
    pub created_at: String,
}

/// A comment in the moderation queue, with the post it's on.
#[derive(sqlx::FromRow)]
pub struct QueuedComment {
    pub id: i64,
    pub author_name: String,
    pub author_email: String,
    pub content: String,
    pub created_at: String,
    pub post_title: String,
    pub slug: String,
}

/// A comment about to be saved.
pub struct NewComment<'a> {
    pub post_id: i64,
    pub author_name: &'a str,
    pub author_email: &'a str,
    pub content: &'a str,
    /// `pending` or `approved`.
    pub status: &'a str,
    /// When it was written; `None` means now. Imports keep the old date.
    pub created_at: Option<String>,
}

// Comments on posts in the trash aren't listed, so they aren't counted either.
const NOT_IN_TRASH: &str = "post_id NOT IN (SELECT id FROM posts WHERE trashed_at IS NOT NULL)";

/// How many comments have each status (`pending`, `approved`, `spam`).
pub async fn count_by_status(pool: &SqlitePool) -> HashMap<String, i64> {
    or_log(
        sqlx::query_as(&format!(
            "SELECT status, COUNT(*) FROM comments WHERE {NOT_IN_TRASH} GROUP BY status"
        ))
        .fetch_all(pool)
        .await,
        "count comments",
    )
    .into_iter()
    .collect()
}

/// How many comments wait for approval (the number on the menu badge).
pub async fn count_waiting(pool: &SqlitePool) -> i64 {
    or_log(
        sqlx::query_scalar(&format!(
            "SELECT COUNT(*) FROM comments WHERE status = 'pending' AND {NOT_IN_TRASH}"
        ))
        .fetch_one(pool)
        .await,
        "count waiting comments",
    )
}

/// The newest 200 comments with this status, for the moderation screen.
pub async fn queue(pool: &SqlitePool, status: &str) -> Vec<QueuedComment> {
    or_log(
        sqlx::query_as(
            "SELECT c.id, c.author_name, c.author_email, c.content, c.created_at,
                    p.title AS post_title, p.slug
             FROM comments c JOIN posts p ON p.id = c.post_id
             WHERE c.status = ? AND p.trashed_at IS NULL ORDER BY c.id DESC LIMIT 200",
        )
        .bind(status)
        .fetch_all(pool)
        .await,
        "list comments",
    )
}

/// Approved comments on one post, oldest first.
pub async fn approved_on(pool: &SqlitePool, post_id: i64) -> Vec<Comment> {
    or_log(
        sqlx::query_as(
            "SELECT id, post_id, author_name, author_email, content, created_at FROM comments
             WHERE post_id = ? AND status = 'approved' ORDER BY id",
        )
        .bind(post_id)
        .fetch_all(pool)
        .await,
        "load comments",
    )
}

/// Every approved comment, oldest first (for the export).
pub async fn all_approved(pool: &SqlitePool) -> Vec<Comment> {
    or_log(
        sqlx::query_as(
            "SELECT id, post_id, author_name, author_email, content, created_at FROM comments
             WHERE status = 'approved' ORDER BY created_at",
        )
        .fetch_all(pool)
        .await,
        "load comments for export",
    )
}

/// Save a comment and return its id.
pub async fn add(pool: &SqlitePool, comment: &NewComment<'_>) -> Result<i64, sqlx::Error> {
    let result = sqlx::query(
        "INSERT INTO comments (post_id, author_name, author_email, content, status, created_at)
         VALUES (?, ?, ?, ?, ?, COALESCE(?, CURRENT_TIMESTAMP))",
    )
    .bind(comment.post_id)
    .bind(comment.author_name)
    .bind(comment.author_email)
    .bind(comment.content)
    .bind(comment.status)
    .bind(&comment.created_at)
    .execute(pool)
    .await?;
    Ok(result.last_insert_rowid())
}

/// Move a comment to `pending`, `approved` or `spam`. `false` when there is
/// no such comment.
pub async fn set_status(pool: &SqlitePool, id: i64, status: &str) -> Result<bool, sqlx::Error> {
    let result = sqlx::query("UPDATE comments SET status = ? WHERE id = ?")
        .bind(status)
        .bind(id)
        .execute(pool)
        .await?;
    Ok(result.rows_affected() > 0)
}

/// Remove a comment for good. `false` when there is no such comment.
pub async fn delete(pool: &SqlitePool, id: i64) -> Result<bool, sqlx::Error> {
    let result = sqlx::query("DELETE FROM comments WHERE id = ?")
        .bind(id)
        .execute(pool)
        .await?;
    Ok(result.rows_affected() > 0)
}
