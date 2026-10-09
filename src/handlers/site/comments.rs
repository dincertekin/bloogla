//! Reader comments: the form under each post, and the approved comments
//! shown there. Moderation is in `handlers/admin/comments.rs`.

use crate::app::state::AppState;
use crate::content::markdown::excerpt;
use crate::content::text::{display_date, escape_html};
use crate::db::or_log;
use crate::db::posts::PUBLIC_POST_FILTER;
use crate::db::settings::{self, CommentMode};
use crate::i18n::Lang;
use crate::server::routes::post_path;

use axum::extract::{ConnectInfo, Path, State};
use axum::http::{HeaderMap, StatusCode};
use axum::response::{IntoResponse, Redirect, Response};
use axum_extra::extract::Form;
use serde::{Deserialize, Serialize};
use std::net::SocketAddr;

/// Humans take longer than this between opening a post and sending a comment.
const MIN_SECONDS_TO_WRITE: i64 = 3;
/// Comments with more links than this always wait for approval.
const MAX_LINKS_WITHOUT_REVIEW: usize = 2;

/// An approved comment, ready for themes.
#[derive(Serialize)]
pub struct PublicComment {
    pub id: i64,
    pub author: String,
    pub date: String,
    /// Escaped text with paragraphs; safe to print with `| safe`.
    pub html: String,
}

/// Approved comments on a post, oldest first.
pub async fn approved_for(state: &AppState, post_id: i64, lang: Lang) -> Vec<PublicComment> {
    let tz = crate::db::settings::load(&state.pool).await.timezone;
    let rows: Vec<(i64, String, String, String)> = or_log(
        sqlx::query_as(
            "SELECT id, author_name, content, created_at FROM comments
             WHERE post_id = ? AND status = 'approved' ORDER BY id",
        )
        .bind(post_id)
        .fetch_all(&state.pool)
        .await,
        "load comments",
    );
    rows.into_iter()
        .map(|(id, author, content, created_at)| PublicComment {
            id,
            author,
            date: display_date(lang, tz, &created_at),
            html: paragraphs(&content),
        })
        .collect()
}

/// Plain text to escaped HTML paragraphs (blank lines split paragraphs).
pub fn paragraphs(text: &str) -> String {
    text.split("\n\n")
        .map(str::trim)
        .filter(|p| !p.is_empty())
        .map(|p| format!("<p>{}</p>", escape_html(p).replace('\n', "<br>")))
        .collect()
}

/// Message shown above the comment form after submitting.
pub fn notice(code: Option<&str>) -> Option<&'static str> {
    match code? {
        "pending" => Some("Thanks! Your comment will appear once it's approved."),
        "slow" => Some("You've sent several comments in a short time. Please wait a few minutes."),
        "invalid" => Some("Please add your name and a comment (up to 5,000 characters)."),
        _ => None,
    }
}

#[derive(Deserialize)]
pub struct CommentForm {
    name: String,
    #[serde(default)]
    email: String,
    content: String,
    /// Hidden field; people never fill it in, many bots do.
    #[serde(default)]
    website: String,
    /// Set by the page's script when the post was opened (Unix milliseconds).
    #[serde(default)]
    ts: String,
}

/// POST /post/:slug/comments -> Add a comment.
pub async fn submit(
    State(state): State<AppState>,
    Path(slug): Path<String>,
    ConnectInfo(peer): ConnectInfo<SocketAddr>,
    headers: HeaderMap,
    Form(form): Form<CommentForm>,
) -> Response {
    let mode = settings::load(&state.pool).await.comments;
    let post_id: Option<i64> = or_log(
        sqlx::query_scalar(&format!(
            "SELECT id FROM posts WHERE slug = ? AND is_page = 0 AND {PUBLIC_POST_FILTER}"
        ))
        .bind(&slug)
        .fetch_optional(&state.pool)
        .await,
        "find post for comment",
    );
    let (Some(post_id), false) = (post_id, mode == CommentMode::Off) else {
        return StatusCode::NOT_FOUND.into_response();
    };

    let back = |query: &str, anchor: &str| {
        let path = post_path(&slug, false);
        Redirect::to(&format!("{path}{query}#{anchor}")).into_response()
    };

    // Bots: act as if it worked, store nothing.
    let opened_at = form.ts.trim().parse::<i64>().ok();
    let too_fast =
        opened_at.is_some_and(|t| chrono::Utc::now().timestamp() - t / 1000 < MIN_SECONDS_TO_WRITE);
    if !form.website.trim().is_empty() || too_fast {
        return back("?comment=pending", "comments");
    }

    let name = form.name.trim();
    let content = form.content.trim();
    let email = form.email.trim();
    let valid = (1..=80).contains(&name.chars().count())
        && (2..=5000).contains(&content.chars().count())
        && email.len() <= 200
        && (email.is_empty() || email.contains('@'));
    if !valid {
        return back("?comment=invalid", "comments");
    }

    if !super::allow_from(&state, peer, &headers) {
        return back("?comment=slow", "comments");
    }

    let links = content.matches("http://").count() + content.matches("https://").count();
    let needs_review =
        mode == CommentMode::Moderated || opened_at.is_none() || links > MAX_LINKS_WITHOUT_REVIEW;
    let status = if needs_review { "pending" } else { "approved" };

    let id = match sqlx::query(
        "INSERT INTO comments (post_id, author_name, author_email, content, status)
         VALUES (?, ?, ?, ?, ?)",
    )
    .bind(post_id)
    .bind(name)
    .bind(email)
    .bind(content)
    .bind(status)
    .execute(&state.pool)
    .await
    {
        Ok(r) => r.last_insert_rowid(),
        Err(e) => {
            tracing::error!("Failed to save comment: {e}");
            return StatusCode::INTERNAL_SERVER_ERROR.into_response();
        }
    };

    if needs_review {
        let title: String = or_log(
            sqlx::query_scalar("SELECT title FROM posts WHERE id = ?")
                .bind(post_id)
                .fetch_one(&state.pool)
                .await,
            "comment post title",
        );
        crate::services::email::notify_new_comment(
            &state,
            title,
            name.to_string(),
            excerpt(content, 300),
        );
        back("?comment=pending", "comments")
    } else {
        back("", &format!("comment-{id}"))
    }
}

#[cfg(test)]
mod tests {
    use super::paragraphs;

    #[test]
    fn comment_text_is_escaped_into_paragraphs() {
        assert_eq!(
            paragraphs("Hi <b>there</b>\nline two\n\n\nNext & last"),
            "<p>Hi &lt;b&gt;there&lt;/b&gt;<br>line two</p><p>Next &amp; last</p>"
        );
    }
}
