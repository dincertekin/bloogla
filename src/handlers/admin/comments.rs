//! Comment moderation: approve, mark as spam, or delete.

use super::StatusFilter;
use crate::app::models::CurrentUser;
use crate::app::state::AppState;
use crate::content::text::{display_datetime, escape_html};
use crate::db::or_log;
use crate::db::settings::{self, CommentMode};
use crate::handlers::site::comments::paragraphs;
use crate::server::routes::post_path;

use askama::Template;
use axum::extract::{Extension, Path, Query, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use serde::Deserialize;
use std::collections::HashMap;

/// A comment in the moderation queue.
pub struct CommentRow {
    pub id: i64,
    /// Set for webmentions: the page that linked to the post.
    pub source_url: Option<String>,
    pub author: String,
    pub email: String,
    /// Escaped text with paragraphs.
    pub html: String,
    pub date: String,
    /// Escaped link to the post's comments, for filling into sentences.
    pub post_link: String,
}

/// Comment moderation page.
#[derive(Template)]
#[template(path = "comments.html")]
pub struct CommentsTemplate {
    pub blog_name: String,
    pub me: CurrentUser,
    pub comments: Vec<CommentRow>,
    pub filters: Vec<StatusFilter>,
    pub active_filter: String,
    pub mode_off: bool,
}

/// A comment joined with its post, as read for the moderation queue.
#[derive(sqlx::FromRow)]
struct QueueRow {
    id: i64,
    author_name: String,
    author_email: String,
    content: String,
    created_at: String,
    post_title: String,
    slug: String,
    source_url: Option<String>,
}

#[derive(Deserialize)]
pub struct QueueQuery {
    status: Option<String>,
}

/// GET /admin/comments -> Moderation queue.
pub async fn page(
    State(state): State<AppState>,
    Extension(me): Extension<CurrentUser>,
    Query(query): Query<QueueQuery>,
) -> Response {
    let active = query
        .status
        .as_deref()
        .filter(|s| matches!(*s, "approved" | "spam"))
        .unwrap_or("pending");

    let counts: HashMap<String, i64> = or_log(
        sqlx::query_as("SELECT status, COUNT(*) FROM comments GROUP BY status")
            .fetch_all(&state.pool)
            .await,
        "count comments",
    )
    .into_iter()
    .collect();
    let count = |s: &str| counts.get(s).copied().unwrap_or(0) as usize;
    let filters = vec![
        StatusFilter::new("pending", "Waiting", count("pending")),
        StatusFilter::new("approved", "Approved", count("approved")),
        StatusFilter::new("spam", "Spam", count("spam")),
    ];

    let rows: Vec<QueueRow> = or_log(
        sqlx::query_as(
            "SELECT c.id, c.author_name, c.author_email, c.content, c.created_at,
                    p.title AS post_title, p.slug, c.source_url
             FROM comments c JOIN posts p ON p.id = c.post_id
             WHERE c.status = ? ORDER BY c.id DESC LIMIT 200",
        )
        .bind(active)
        .fetch_all(&state.pool)
        .await,
        "list comments",
    );
    let comments = rows
        .into_iter()
        .map(|row| CommentRow {
            id: row.id,
            author: row.author_name,
            email: row.author_email,
            html: paragraphs(&row.content),
            date: display_datetime(me.lang, &row.created_at),
            post_link: format!(
                r#"<a href="{}#comments" target="_blank" style="color: inherit">{}</a>"#,
                post_path(&row.slug, false),
                escape_html(&row.post_title)
            ),
            source_url: row.source_url,
        })
        .collect();

    let site = settings::load(&state.pool).await;
    CommentsTemplate {
        blog_name: site.blog_name.clone(),
        me,
        comments,
        filters,
        active_filter: active.to_string(),
        mode_off: site.comments == CommentMode::Off,
    }
    .into_response()
}

/// POST /admin/comments/:id/approve, /spam, /pending -> Move a comment.
pub async fn set_status(
    State(state): State<AppState>,
    Path((id, action)): Path<(i64, String)>,
) -> StatusCode {
    let status = match action.as_str() {
        "approve" => "approved",
        "spam" => "spam",
        "pending" => "pending",
        _ => return StatusCode::NOT_FOUND,
    };
    match sqlx::query("UPDATE comments SET status = ? WHERE id = ?")
        .bind(status)
        .bind(id)
        .execute(&state.pool)
        .await
    {
        Ok(r) if r.rows_affected() > 0 => StatusCode::OK,
        Ok(_) => StatusCode::NOT_FOUND,
        Err(e) => {
            tracing::error!("Failed to update comment: {e}");
            StatusCode::INTERNAL_SERVER_ERROR
        }
    }
}

/// DELETE /admin/comments/:id -> Remove a comment for good.
pub async fn delete(State(state): State<AppState>, Path(id): Path<i64>) -> StatusCode {
    match sqlx::query("DELETE FROM comments WHERE id = ?")
        .bind(id)
        .execute(&state.pool)
        .await
    {
        Ok(r) if r.rows_affected() > 0 => StatusCode::OK,
        Ok(_) => StatusCode::NOT_FOUND,
        Err(e) => {
            tracing::error!("Failed to delete comment: {e}");
            StatusCode::INTERNAL_SERVER_ERROR
        }
    }
}
