//! Newsletter sign-up (double opt-in), confirmation and unsubscribing.
//! The subscriber list is in `handlers/admin/subscribers.rs`.

use super::render_message;
use crate::app::security::random_hex;
use crate::app::state::AppState;
use crate::content::text::escape_html;
use crate::db::{or_log, settings};
use crate::services::email::{layout, send, Email};

use axum::extract::{ConnectInfo, Query, State};
use axum::http::{HeaderMap, StatusCode};
use axum::response::{IntoResponse, Redirect, Response};
use axum_extra::extract::Form;
use serde::Deserialize;
use std::collections::HashMap;
use std::net::SocketAddr;

#[derive(Deserialize)]
pub struct SubscribeForm {
    email: String,
    /// Hidden field; people leave it empty.
    #[serde(default)]
    website: String,
    /// Page to return to, e.g. `/post/my-post`.
    #[serde(default)]
    return_to: String,
}

#[derive(Deserialize)]
pub struct TokenQuery {
    #[serde(default)]
    token: String,
}

/// Message shown by the subscribe form after sending it.
pub fn notice(code: Option<&str>) -> Option<&'static str> {
    match code? {
        "check" => Some("Almost done: check your inbox for a link to confirm."),
        "invalid" => Some("That email address doesn't look right."),
        "slow" => Some("Too many attempts. Please try again in a few minutes."),
        "error" => Some("Something went wrong sending the confirmation. Please try again later."),
        _ => None,
    }
}

/// A local path to send the reader back to (never another site).
fn safe_return(path: &str) -> &str {
    if path.starts_with('/') && !path.starts_with("//") && !path.contains(['\\', '\n', '\r']) {
        path.split(['?', '#']).next().unwrap_or("/")
    } else {
        "/"
    }
}

/// POST /subscribe -> Start a subscription and email a confirmation link.
pub async fn subscribe(
    State(state): State<AppState>,
    ConnectInfo(peer): ConnectInfo<SocketAddr>,
    headers: HeaderMap,
    Form(form): Form<SubscribeForm>,
) -> Response {
    let site = settings::load(&state.pool).await;
    if !site.newsletter {
        return StatusCode::NOT_FOUND.into_response();
    }
    let back = |result: &str| {
        Redirect::to(&format!(
            "{}?subscribe={result}#subscribe",
            safe_return(&form.return_to)
        ))
        .into_response()
    };

    let email = form.email.trim().to_lowercase();
    if !form.website.trim().is_empty() {
        return back("check");
    }
    if email.len() > 200 || !email.contains('@') || email.contains(char::is_whitespace) {
        return back("invalid");
    }
    if !super::allow_from(&state, peer, &headers) {
        return back("slow");
    }

    // Same answer whether or not the address is already subscribed, so the
    // form can't be used to find out who reads the site.
    let existing: Option<(String, String)> = or_log(
        sqlx::query_as("SELECT status, token FROM subscribers WHERE email = ?")
            .bind(&email)
            .fetch_optional(&state.pool)
            .await,
        "find subscriber",
    );
    let token = match existing {
        Some((status, _)) if status == "active" => return back("check"),
        Some((_, token)) => {
            let _ = sqlx::query("UPDATE subscribers SET status = 'pending' WHERE email = ?")
                .bind(&email)
                .execute(&state.pool)
                .await;
            token
        }
        None => {
            let token = random_hex(20);
            if let Err(e) = sqlx::query("INSERT INTO subscribers (email, token) VALUES (?, ?)")
                .bind(&email)
                .bind(&token)
                .execute(&state.pool)
                .await
            {
                tracing::error!("Failed to add subscriber: {e}");
                return back("error");
            }
            token
        }
    };

    let (blog_name, lang) = (&site.blog_name, site.language);
    let confirm = format!("{}/subscribe/confirm?token={token}", state.config.base_url);
    let ignore = lang.t("If you didn't ask for this, ignore this email and nothing will happen.");
    let message = Email {
        to: email.clone(),
        subject: lang.tv("Confirm your subscription to {site}", blog_name),
        text: format!(
            "{}\n\n{confirm}\n\n{ignore}\n",
            lang.tv(
                "Please confirm you'd like new posts from {site} by email.",
                blog_name
            )
        ),
        html: layout(
            blog_name,
            &format!(
                r#"<p style="margin:0 0 20px">{}</p>
<p style="margin:0 0 20px"><a href="{confirm}" style="display:inline-block;background:#111827;color:#ffffff;text-decoration:none;padding:10px 16px;border-radius:8px;font-size:14px">{}</a></p>"#,
                lang.tv(
                    "Please confirm you'd like new posts from {site} by email.",
                    escape_html(blog_name)
                ),
                lang.t("Confirm subscription"),
            ),
            ignore,
        ),
        unsubscribe: None,
    };
    if let Err(e) = send(&state, &message).await {
        tracing::error!("Confirmation email to {email} failed: {e}");
        return back("error");
    }
    back("check")
}

/// GET /subscribe/confirm -> Activate a subscription.
pub async fn confirm(State(state): State<AppState>, Query(query): Query<TokenQuery>) -> Response {
    let updated = sqlx::query(
        "UPDATE subscribers SET status = 'active', confirmed_at = CURRENT_TIMESTAMP
         WHERE token = ? AND token != ''",
    )
    .bind(&query.token)
    .execute(&state.pool)
    .await
    .map(|r| r.rows_affected())
    .unwrap_or(0);

    if updated == 0 {
        return render_message(
            &state,
            StatusCode::NOT_FOUND,
            "That link didn't work",
            "It may be old or incomplete. Try subscribing again.",
            None,
        )
        .await;
    }
    render_message(
        &state,
        StatusCode::OK,
        "You're subscribed",
        "New posts will arrive in your inbox. Every email has a link to unsubscribe.",
        None,
    )
    .await
}

/// GET /unsubscribe -> Ask before unsubscribing (link scanners also open links).
pub async fn unsubscribe_page(
    State(state): State<AppState>,
    Query(query): Query<TokenQuery>,
) -> Response {
    render_message(
        &state,
        StatusCode::OK,
        "Unsubscribe?",
        "You'll stop getting new posts by email.",
        Some(("/unsubscribe", &query.token, "Unsubscribe")),
    )
    .await
}

/// POST /unsubscribe -> Stop emails. Also handles one-click unsubscribe from
/// mail apps (`List-Unsubscribe-Post`), which send the token in the URL.
pub async fn unsubscribe(
    State(state): State<AppState>,
    Query(query): Query<TokenQuery>,
    form: Option<Form<HashMap<String, String>>>,
) -> Response {
    let token = form
        .as_ref()
        .and_then(|Form(f)| f.get("token").cloned())
        .filter(|t| !t.is_empty())
        .unwrap_or(query.token);
    let _ = sqlx::query(
        "UPDATE subscribers SET status = 'unsubscribed' WHERE token = ? AND token != ''",
    )
    .bind(&token)
    .execute(&state.pool)
    .await;

    render_message(
        &state,
        StatusCode::OK,
        "You're unsubscribed",
        "You won't get any more emails from this site.",
        None,
    )
    .await
}

#[cfg(test)]
mod tests {
    use super::safe_return;

    #[test]
    fn only_returns_to_local_paths() {
        assert_eq!(safe_return("/post/a?x=1#y"), "/post/a");
        assert_eq!(safe_return("//evil.com"), "/");
        assert_eq!(safe_return("https://evil.com"), "/");
        assert_eq!(safe_return("/\\evil.com"), "/");
    }
}
