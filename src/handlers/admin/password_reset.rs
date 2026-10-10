//! "Forgot password?": email a link that lets someone choose a new password.
//!
//! 1. `GET /admin/forgot-password` asks for the email address.
//! 2. `POST` to it emails a link with a random code, valid for an hour and
//!    usable once. Only the code's hash is stored. The page says the same
//!    whether or not the address has an account, so it can't be used to find
//!    out who has one.
//! 3. `GET /admin/reset-password?token=...` asks for the new password, and
//!    `POST` to it saves it and signs the account out everywhere.
//!
//! Without a mail server the page explains who can help instead.

use crate::app::security::{
    hash_password, hash_token, log_event, missing_password_rules, random_hex, PasswordRule,
    PASSWORD_RULES,
};
use crate::app::state::AppState;
use crate::content::text::escape_html;
use crate::db::{or_log, settings};
use crate::i18n::Lang;

use askama::Template;
use axum::extract::{ConnectInfo, Form, Query, State};
use axum::http::{header, HeaderMap, HeaderValue};
use axum::response::{IntoResponse, Redirect, Response};
use serde::Deserialize;
use sqlx::SqlitePool;
use std::net::SocketAddr;

/// How long a reset link works.
const LINK_MINUTES: i64 = 60;

#[derive(Template)]
#[template(path = "forgot_password.html")]
pub struct ForgotTemplate {
    pub lang: Lang,
    /// False when no mail server is set up, so no link can be sent.
    pub email_works: bool,
    /// True after the form was sent.
    pub sent: bool,
    pub error: Option<String>,
}

#[derive(Template)]
#[template(path = "reset_password.html")]
pub struct ResetTemplate {
    pub lang: Lang,
    /// The code from the link; `None` when the link isn't valid (anymore).
    pub token: Option<String>,
    pub password_rules: &'static [PasswordRule],
    pub error: Option<String>,
}

#[derive(Deserialize)]
pub struct ForgotForm {
    email: String,
}

#[derive(Deserialize)]
pub struct TokenQuery {
    #[serde(default)]
    token: String,
}

#[derive(Deserialize)]
pub struct ResetForm {
    token: String,
    password: String,
    confirm_password: String,
}

async fn email_works(state: &AppState) -> bool {
    crate::services::email::smtp_settings(state).await.is_some()
}

/// GET /admin/forgot-password -> Ask for the email address.
pub async fn forgot_page(State(state): State<AppState>) -> Response {
    ForgotTemplate {
        lang: settings::load(&state.pool).await.language,
        email_works: email_works(&state).await,
        sent: false,
        error: None,
    }
    .into_response()
}

/// POST /admin/forgot-password -> Email a reset link, if the account exists.
pub async fn send_link(
    State(state): State<AppState>,
    ConnectInfo(peer): ConnectInfo<SocketAddr>,
    headers: HeaderMap,
    Form(form): Form<ForgotForm>,
) -> Response {
    let lang = settings::load(&state.pool).await.language;
    let email_works = email_works(&state).await;
    let page = |sent, error: Option<&str>| {
        ForgotTemplate {
            lang,
            email_works,
            sent,
            error: error.map(|e| lang.t_owned(e)),
        }
        .into_response()
    };
    if !email_works {
        return page(false, None);
    }
    // The same limit as public forms, so nobody can flood an inbox.
    if !crate::handlers::site::allow_from(&state, peer, &headers) {
        return page(
            false,
            Some("Too many tries from your connection. Wait a few minutes and try again."),
        );
    }

    let email = form.email.trim().to_lowercase();
    let user: Option<(i64, i64)> = or_log(
        sqlx::query_as("SELECT id, session_version FROM users WHERE email = ? COLLATE NOCASE")
            .bind(&email)
            .fetch_optional(&state.pool)
            .await,
        "find account for password reset",
    );
    if let Some((user_id, version)) = user {
        match create_link_if_current(&state.pool, user_id, version).await {
            Ok(Some(token)) => {
                let link = format!(
                    "{}/admin/reset-password?token={token}",
                    state.config.base_url
                );
                send_email(&state, email.clone(), link);
                log_event("password_reset_requested", &[("user", &email)]);
            }
            Ok(None) => {} // The password or recovery address changed meanwhile.
            Err(e) => tracing::error!("Could not save password reset link: {e}"),
        }
    }
    page(true, None)
}

/// Save a new reset code for `user_id` (replacing older ones) and return it.
#[cfg(test)]
pub async fn create_link(pool: &SqlitePool, user_id: i64) -> Result<String, sqlx::Error> {
    let version = sqlx::query_scalar("SELECT session_version FROM users WHERE id = ?")
        .bind(user_id)
        .fetch_one(pool)
        .await?;
    create_link_if_current(pool, user_id, version)
        .await?
        .ok_or(sqlx::Error::RowNotFound)
}

/// An in-flight request to the former recovery address must not issue a
/// usable link after a password or email change. Take the write lock first;
/// stale requests must not even delete a newer request's link.
pub async fn create_link_if_current(
    pool: &SqlitePool,
    user_id: i64,
    version: i64,
) -> Result<Option<String>, sqlx::Error> {
    let token = random_hex(32);
    let mut tx = pool.begin().await?;
    sqlx::query(
        "DELETE FROM password_resets
         WHERE (user_id = ? OR expires_at < datetime('now'))
         AND EXISTS (SELECT 1 FROM users WHERE id = ? AND session_version = ?)",
    )
    .bind(user_id)
    .bind(user_id)
    .bind(version)
    .execute(&mut *tx)
    .await?;
    let inserted = sqlx::query(
        "INSERT INTO password_resets (token_hash, user_id, expires_at)
         SELECT ?, id, datetime('now', ?) FROM users WHERE id = ? AND session_version = ?",
    )
    .bind(hash_token(&token))
    .bind(format!("+{LINK_MINUTES} minutes"))
    .bind(user_id)
    .bind(version)
    .execute(&mut *tx)
    .await?
    .rows_affected();
    tx.commit().await?;
    Ok((inserted == 1).then_some(token))
}

/// Send the link in the background, so the page answers just as fast for
/// addresses without an account.
fn send_email(state: &AppState, to: String, link: String) {
    let state = state.clone();
    tokio::spawn(async move {
        let site = settings::load(&state.pool).await;
        let (lang, blog_name) = (site.language, site.blog_name.clone());
        let text = format!(
            "{}\n\n{link}\n\n{}\n",
            lang.tv("Someone asked to reset your password on {site}.", &blog_name),
            lang.t("The link works for one hour. If it wasn't you, ignore this email: your password stays the same.")
        );
        let html = crate::services::email::layout(
            &blog_name,
            &format!(
                r#"<p style="margin:0 0 16px">{}</p>
<p style="margin:0 0 20px"><a href="{}" style="display:inline-block;background:#111827;color:#ffffff;text-decoration:none;padding:10px 16px;border-radius:8px;font-size:14px">{}</a></p>
<p style="margin:0;color:#6b7280;font-size:14px">{}</p>"#,
                lang.tv("Someone asked to reset your password on {site}.", escape_html(&blog_name)),
                escape_html(&link),
                lang.t("Choose a new password"),
                lang.t("The link works for one hour. If it wasn't you, ignore this email: your password stays the same."),
            ),
            "",
        );
        let email = crate::services::email::Email {
            to,
            subject: lang.tv("Reset your password on {site}", &blog_name),
            text,
            html,
        };
        if let Err(e) = crate::services::email::send(&state, &email).await {
            tracing::warn!("Password reset email to {} failed: {e}", email.to);
        }
    });
}

/// The account a reset code belongs to, if the code is valid and unused.
async fn user_for(pool: &SqlitePool, token: &str) -> Option<i64> {
    if token.len() != 64 {
        return None;
    }
    or_log(
        sqlx::query_scalar(
            "SELECT user_id FROM password_resets
             WHERE token_hash = ? AND expires_at > datetime('now')",
        )
        .bind(hash_token(token))
        .fetch_optional(pool)
        .await,
        "check password reset link",
    )
}

/// Reset pages carry the code in their address: never keep them, and never
/// pass the address on to other sites.
fn private(mut response: Response) -> Response {
    let headers = response.headers_mut();
    headers.insert(header::CACHE_CONTROL, HeaderValue::from_static("no-store"));
    headers.insert(
        header::REFERRER_POLICY,
        HeaderValue::from_static("no-referrer"),
    );
    response
}

/// GET /admin/reset-password?token=... -> Choose a new password.
pub async fn reset_page(
    State(state): State<AppState>,
    Query(query): Query<TokenQuery>,
) -> Response {
    let valid = user_for(&state.pool, &query.token).await.is_some();
    private(
        ResetTemplate {
            lang: settings::load(&state.pool).await.language,
            token: valid.then_some(query.token),
            password_rules: PASSWORD_RULES,
            error: None,
        }
        .into_response(),
    )
}

/// POST /admin/reset-password -> Save the new password.
pub async fn reset(State(state): State<AppState>, Form(form): Form<ResetForm>) -> Response {
    let lang = settings::load(&state.pool).await.language;
    let invalid = || {
        private(
            ResetTemplate {
                lang,
                token: None,
                password_rules: PASSWORD_RULES,
                error: None,
            }
            .into_response(),
        )
    };
    if user_for(&state.pool, &form.token).await.is_none() {
        return invalid();
    }
    let error = |message: String| {
        private(
            ResetTemplate {
                lang,
                token: Some(form.token.clone()),
                password_rules: PASSWORD_RULES,
                error: Some(message),
            }
            .into_response(),
        )
    };
    let missing = missing_password_rules(&form.password);
    if !missing.is_empty() {
        let rules: Vec<String> = missing.iter().map(|r| lang.t_owned(r.text)).collect();
        return error(lang.tv("Your password still needs: {rules}", rules.join(", ")));
    }
    if form.password != form.confirm_password {
        return error(lang.t("The passwords don't match.").to_string());
    }

    let password = form.password.clone();
    let Ok(Ok(hash)) = tokio::task::spawn_blocking(move || hash_password(&password)).await else {
        return error(
            lang.t("Couldn't save the password. Please try again.")
                .to_string(),
        );
    };
    // Recheck and consume the link after hashing: another request may have
    // used or replaced it while that slow work ran.
    let user_id = match crate::db::users::reset_password(
        &state.pool,
        &hash_token(&form.token),
        &hash,
    )
    .await
    {
        Ok(Some(id)) => id,
        Ok(None) => return invalid(),
        Err(e) => {
            tracing::error!("Could not reset password: {e}");
            return error(
                lang.t("Couldn't save the password. Please try again.")
                    .to_string(),
            );
        }
    };
    let email: String = or_log(
        sqlx::query_scalar("SELECT email FROM users WHERE id = ?")
            .bind(user_id)
            .fetch_one(&state.pool)
            .await,
        "load account email",
    );
    log_event("password_reset", &[("user", &email)]);
    private(Redirect::to("/admin/login?reset=1").into_response())
}
