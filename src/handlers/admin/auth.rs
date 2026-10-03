//! Signing in and out of the admin panel.

use crate::app::models::{SESSION_USER_ID, SESSION_VERSION};
use crate::app::security::log_event;
use crate::app::state::AppState;
use crate::db::settings;
use crate::i18n::Lang;

use askama::Template;
use axum::extract::{ConnectInfo, Form, State};
use axum::http::HeaderMap;
use axum::response::{IntoResponse, Redirect, Response};
use serde::Deserialize;
use std::net::SocketAddr;
use tower_sessions::Session;

#[derive(Template)]
#[template(path = "login.html")]
pub struct LoginTemplate {
    pub lang: Lang,
    pub error: Option<String>,
}

#[derive(Deserialize)]
pub struct LoginForm {
    email: String,
    password: String,
}

/// GET /admin/login -> Sign-in form.
pub async fn login_page(State(state): State<AppState>) -> impl IntoResponse {
    LoginTemplate {
        lang: settings::load(&state.pool).await.language,
        error: None,
    }
}

/// POST /admin/login -> Check the email and password, then sign in.
pub async fn login(
    State(state): State<AppState>,
    ConnectInfo(peer): ConnectInfo<SocketAddr>,
    headers: HeaderMap,
    session: Session,
    Form(form): Form<LoginForm>,
) -> Response {
    let ip = crate::handlers::client_ip(&state, peer, &headers).to_string();
    let email = form.email.trim().to_string();
    let user: Option<(i64, String, i64)> = sqlx::query_as(
        "SELECT id, password_hash, session_version FROM users WHERE email = ? COLLATE NOCASE",
    )
    .bind(&email)
    .fetch_optional(&state.pool)
    .await
    .unwrap_or_else(|e| {
        tracing::error!("Failed to look up login: {e}");
        None
    });

    // Password hashing is slow on purpose, so it runs off the async threads.
    let password = form.password;
    let stored_hash = user.as_ref().map(|(_, hash, _)| hash.clone());
    let valid = tokio::task::spawn_blocking(move || {
        crate::app::security::verify_password(&password, stored_hash.as_deref())
    })
    .await
    .unwrap_or(false);

    if let (true, Some((user_id, _, version))) = (valid, user) {
        log_event("login", &[("user", &email), ("ip", &ip)]);
        sign_in(&session, user_id, version).await;
        return Redirect::to("/admin").into_response();
    }
    log_event("login_failed", &[("email", &email), ("ip", &ip)]);

    let lang = settings::load(&state.pool).await.language;
    LoginTemplate {
        lang,
        error: Some(lang.t("Invalid email or password.").into()),
    }
    .into_response()
}

/// Make `session` signed in as the account with `user_id`.
pub async fn sign_in(session: &Session, user_id: i64, session_version: i64) {
    // A new session id on sign-in, so an id set before can't be reused.
    if let Err(e) = session.cycle_id().await {
        tracing::error!("Failed to cycle session id: {e}");
    }
    let saved = async {
        session.insert(SESSION_USER_ID, user_id).await?;
        session.insert(SESSION_VERSION, session_version).await
    }
    .await;
    if let Err(e) = saved {
        tracing::error!("Failed to save session: {e}");
    }
}

/// GET /admin/logout -> End the session and go back to the sign-in form.
pub async fn logout(session: Session) -> Redirect {
    let _ = session.flush().await;
    Redirect::to("/admin/login")
}
