//! Comment moderation: approve, mark as spam, or delete.

use super::StatusFilter;
use crate::app::models::CurrentUser;
use crate::app::state::AppState;
use crate::content::text::{display_datetime, escape_html};
use crate::db::comments;
use crate::db::settings::{self, CommentMode};
use crate::handlers::site::comments::paragraphs;
use crate::server::routes::post_path;

use askama::Template;
use axum::extract::{Extension, Path, Query, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use serde::Deserialize;

/// A comment in the moderation queue.
pub struct CommentRow {
    pub id: i64,
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

    let counts = comments::count_by_status(&state.pool).await;
    let count = |s: &str| counts.get(s).copied().unwrap_or(0) as usize;
    let filters = vec![
        StatusFilter::new("pending", "Waiting", count("pending")),
        StatusFilter::new("approved", "Approved", count("approved")),
        StatusFilter::new("spam", "Spam", count("spam")),
    ];

    let rows = comments::queue(&state.pool, active).await;
    let tz = settings::load(&state.pool).await.timezone;
    let queue = rows
        .into_iter()
        .map(|row| CommentRow {
            id: row.id,
            author: row.author_name,
            email: row.author_email,
            html: paragraphs(&row.content),
            date: display_datetime(me.lang, tz, &row.created_at),
            post_link: format!(
                r#"<a href="{}#comments" target="_blank" style="color: inherit">{}</a>"#,
                post_path(&row.slug, false),
                escape_html(&row.post_title)
            ),
        })
        .collect();

    let site = settings::load(&state.pool).await;
    CommentsTemplate {
        blog_name: site.blog_name.clone(),
        me,
        comments: queue,
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
    match comments::set_status(&state.pool, id, status).await {
        Ok(true) => StatusCode::OK,
        Ok(false) => StatusCode::NOT_FOUND,
        Err(e) => {
            tracing::error!("Failed to update comment: {e}");
            StatusCode::INTERNAL_SERVER_ERROR
        }
    }
}

/// DELETE /admin/comments/:id -> Remove a comment for good.
pub async fn delete(State(state): State<AppState>, Path(id): Path<i64>) -> StatusCode {
    match comments::delete(&state.pool, id).await {
        Ok(true) => StatusCode::OK,
        Ok(false) => StatusCode::NOT_FOUND,
        Err(e) => {
            tracing::error!("Failed to delete comment: {e}");
            StatusCode::INTERNAL_SERVER_ERROR
        }
    }
}
