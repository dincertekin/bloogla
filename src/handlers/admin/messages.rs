//! Messages: what visitors send with the contact form (`[contact-form]` in
//! a page). Read here, answered by email, deleted when done.

use crate::app::models::CurrentUser;
use crate::app::state::AppState;
use crate::content::text::{display_datetime, escape_html, url_encode};
use crate::db::or_log;
use crate::db::settings;
use crate::handlers::site::comments::paragraphs;
use crate::server::routes::post_path;

use askama::Template;
use axum::extract::{Extension, Path, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};

/// A message in the list.
pub struct MessageRow {
    pub id: i64,
    pub name: String,
    pub email: String,
    /// Escaped text with paragraphs.
    pub html: String,
    pub date: String,
    /// Not seen before this visit to Messages.
    pub is_new: bool,
    /// Opens the person's email program, addressed to the visitor.
    pub reply_link: String,
    /// Escaped link to the page the form is on (empty if it's gone).
    pub page_link: String,
}

#[derive(Template)]
#[template(path = "messages.html")]
pub struct MessagesTemplate {
    pub blog_name: String,
    pub me: CurrentUser,
    pub messages: Vec<MessageRow>,
}

/// A message joined with its page, as read for the list.
#[derive(sqlx::FromRow)]
struct Row {
    id: i64,
    name: String,
    email: String,
    message: String,
    created_at: String,
    read_at: Option<String>,
    title: Option<String>,
    slug: Option<String>,
    is_page: Option<bool>,
}

/// GET /admin/messages -> Every message, newest first. Opening the list
/// counts as reading them.
pub async fn page(
    State(state): State<AppState>,
    Extension(me): Extension<CurrentUser>,
) -> Response {
    let rows: Vec<Row> = or_log(
        sqlx::query_as(
            "SELECT m.id, m.name, m.email, m.message, m.created_at, m.read_at,
                    p.title, p.slug, p.is_page
             FROM messages m LEFT JOIN posts p ON p.id = m.post_id
             ORDER BY m.id DESC",
        )
        .fetch_all(&state.pool)
        .await,
        "list messages",
    );
    or_log(
        sqlx::query("UPDATE messages SET read_at = CURRENT_TIMESTAMP WHERE read_at IS NULL")
            .execute(&state.pool)
            .await
            .map(|_| ()),
        "mark messages read",
    );

    let site = settings::load(&state.pool).await;
    let subject = me.tv("Your message on {site}", &site.blog_name);
    let messages = rows
        .into_iter()
        .map(|row| MessageRow {
            id: row.id,
            html: paragraphs(&row.message),
            date: display_datetime(me.lang, site.timezone, &row.created_at),
            is_new: row.read_at.is_none(),
            reply_link: format!(
                "mailto:{}?subject={}",
                url_encode(&row.email),
                url_encode(&subject)
            ),
            page_link: match (row.title, row.slug, row.is_page) {
                (Some(title), Some(slug), Some(is_page)) => format!(
                    r#"<a href="{}" target="_blank" style="color: inherit">{}</a>"#,
                    post_path(&slug, is_page),
                    escape_html(&title)
                ),
                _ => String::new(),
            },
            name: row.name,
            email: row.email,
        })
        .collect();

    MessagesTemplate {
        blog_name: site.blog_name.clone(),
        me,
        messages,
    }
    .into_response()
}

/// DELETE /admin/messages/:id -> Delete a message.
pub async fn delete(State(state): State<AppState>, Path(id): Path<i64>) -> StatusCode {
    match sqlx::query("DELETE FROM messages WHERE id = ?")
        .bind(id)
        .execute(&state.pool)
        .await
    {
        Ok(r) if r.rows_affected() > 0 => StatusCode::OK,
        Ok(_) => StatusCode::NOT_FOUND,
        Err(e) => {
            tracing::error!("Failed to delete a message: {e}");
            StatusCode::INTERNAL_SERVER_ERROR
        }
    }
}
