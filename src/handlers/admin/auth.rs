//! Signing in and out of the admin panel.

use crate::app::models::{SESSION_USER_ID, SESSION_VERSION};
use crate::app::security::log_event;
use crate::app::state::AppState;
use crate::db::settings;
use crate::i18n::Lang;

use askama::Template;
use axum::extract::{ConnectInfo, Form, State};
use axum::http::{HeaderMap, StatusCode};
use axum::response::{IntoResponse, Redirect, Response};
use serde::Deserialize;
use std::collections::{HashMap, VecDeque};
use std::net::SocketAddr;
use std::sync::Mutex;
use std::time::{Duration, Instant};
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

/// Failed sign-ins one account may have in [`FAILURE_WINDOW`] before it's slowed down.
const FAILURES_ALLOWED: usize = 5;
const FAILURE_WINDOW: Duration = Duration::from_secs(15 * 60);
/// Once slowed down, an account accepts one attempt per this long, from any address.
const SLOWDOWN: Duration = Duration::from_secs(60);

/// Recent failed sign-ins per email address.
///
/// Login is also rate-limited per visitor address, but that doesn't stop slow
/// guessing spread over many addresses. Accounts are slowed down rather than
/// locked: a lock would let anyone lock the owner out by typing their email.
#[derive(Default)]
struct FailedLogins(HashMap<String, VecDeque<Instant>>);

impl FailedLogins {
    /// Forget failures older than the window.
    fn prune(&mut self, now: Instant) {
        self.0.retain(|_, times| {
            times.retain(|t| now.duration_since(*t) < FAILURE_WINDOW);
            !times.is_empty()
        });
    }

    /// True when this account had too many failures lately and the last one
    /// was less than [`SLOWDOWN`] ago.
    fn must_wait(&mut self, email: &str, now: Instant) -> bool {
        self.prune(now);
        self.0.get(email).is_some_and(|times| {
            times.len() >= FAILURES_ALLOWED
                && times
                    .back()
                    .is_some_and(|last| now.duration_since(*last) < SLOWDOWN)
        })
    }

    fn record(&mut self, email: &str, now: Instant) {
        self.0.entry(email.to_string()).or_default().push_back(now);
    }

    fn clear(&mut self, email: &str) {
        self.0.remove(email);
    }
}

static FAILED_LOGINS: Mutex<Option<FailedLogins>> = Mutex::new(None);

/// Run `f` on the shared failed-login record.
fn failed_logins<T>(f: impl FnOnce(&mut FailedLogins) -> T) -> T {
    let mut guard = FAILED_LOGINS
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    f(guard.get_or_insert_with(FailedLogins::default))
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
    let email = form.email.trim().to_lowercase();

    if failed_logins(|f| f.must_wait(&email, Instant::now())) {
        log_event("login_throttled", &[("email", &email), ("ip", &ip)]);
        let lang = settings::load(&state.pool).await.language;
        return (
            StatusCode::TOO_MANY_REQUESTS,
            LoginTemplate {
                lang,
                error: Some(
                    lang.t(
                        "Too many failed sign-ins for this account. Wait a minute and try again.",
                    )
                    .into(),
                ),
            },
        )
            .into_response();
    }
    let user: Option<(i64, String, i64, bool)> = sqlx::query_as(
        "SELECT id, password_hash, session_version, totp_secret IS NOT NULL
         FROM users WHERE email = ? COLLATE NOCASE",
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
    let stored_hash = user.as_ref().map(|(_, hash, _, _)| hash.clone());
    let valid = tokio::task::spawn_blocking(move || {
        crate::app::security::verify_password(&password, stored_hash.as_deref())
    })
    .await
    .unwrap_or(false);

    if let (true, Some((user_id, _, version, two_factor))) = (valid, user) {
        if two_factor {
            // The password was right; now ask for the code from the app.
            let waiting = async {
                session.cycle_id().await?;
                session.insert(PENDING_USER_ID, user_id).await?;
                session.insert(PENDING_SINCE, unix_now()).await
            }
            .await;
            if let Err(e) = waiting {
                tracing::error!("Failed to save session: {e}");
            }
            return Redirect::to("/admin/login/code").into_response();
        }
        failed_logins(|f| f.clear(&email));
        log_event("login", &[("user", &email), ("ip", &ip)]);
        sign_in(&session, user_id, version).await;
        return Redirect::to("/admin").into_response();
    }
    failed_logins(|f| f.record(&email, Instant::now()));
    log_event("login_failed", &[("email", &email), ("ip", &ip)]);

    let lang = settings::load(&state.pool).await.language;
    LoginTemplate {
        lang,
        error: Some(lang.t("Invalid email or password.").into()),
    }
    .into_response()
}

// ---- Second step for people with two-factor login ----

/// Session keys while the password was right but the code isn't entered yet.
const PENDING_USER_ID: &str = "pending_user_id";
const PENDING_SINCE: &str = "pending_since";
/// How long the code page stays valid after the password.
const PENDING_SECONDS: u64 = 5 * 60;

fn unix_now() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_secs())
}

/// The account waiting for its code, if the password was entered recently.
async fn pending_user(session: &Session) -> Option<i64> {
    let since: u64 = session.get(PENDING_SINCE).await.ok().flatten()?;
    if unix_now().saturating_sub(since) > PENDING_SECONDS {
        return None;
    }
    session.get(PENDING_USER_ID).await.ok().flatten()
}

#[derive(Template)]
#[template(path = "login_code.html")]
pub struct LoginCodeTemplate {
    pub lang: Lang,
    pub error: Option<String>,
}

#[derive(Deserialize)]
pub struct CodeForm {
    code: String,
}

/// GET /admin/login/code -> Ask for the code from the authenticator app.
pub async fn code_page(State(state): State<AppState>, session: Session) -> Response {
    if pending_user(&session).await.is_none() {
        return Redirect::to("/admin/login").into_response();
    }
    LoginCodeTemplate {
        lang: settings::load(&state.pool).await.language,
        error: None,
    }
    .into_response()
}

/// POST /admin/login/code -> Check the code (or a recovery code) and sign in.
pub async fn check_code(
    State(state): State<AppState>,
    ConnectInfo(peer): ConnectInfo<SocketAddr>,
    headers: HeaderMap,
    session: Session,
    Form(form): Form<CodeForm>,
) -> Response {
    let Some(user_id) = pending_user(&session).await else {
        return Redirect::to("/admin/login").into_response();
    };
    let ip = crate::handlers::client_ip(&state, peer, &headers).to_string();
    let lang = settings::load(&state.pool).await.language;
    let user: Option<(String, i64)> =
        sqlx::query_as("SELECT email, session_version FROM users WHERE id = ?")
            .bind(user_id)
            .fetch_optional(&state.pool)
            .await
            .unwrap_or(None);
    let Some((email, version)) = user else {
        return Redirect::to("/admin/login").into_response();
    };
    let email = email.to_lowercase();
    let show_error = |status: StatusCode, message: &'static str| {
        (
            status,
            LoginCodeTemplate {
                lang,
                error: Some(lang.t(message).into()),
            },
        )
            .into_response()
    };

    if failed_logins(|f| f.must_wait(&email, Instant::now())) {
        log_event("login_throttled", &[("email", &email), ("ip", &ip)]);
        return show_error(
            StatusCode::TOO_MANY_REQUESTS,
            "Too many failed sign-ins for this account. Wait a minute and try again.",
        );
    }

    let typed = form.code.trim();
    let mut accepted = false;
    if let Some((secret, last_step)) = crate::db::users::two_factor(&state.pool, user_id).await {
        if let Some(step) = crate::app::totp::verify(&secret, typed, unix_now(), last_step) {
            crate::db::users::set_two_factor_step(&state.pool, user_id, step).await;
            accepted = true;
        }
    }
    // A recovery code is longer than the six digits from the app.
    if !accepted
        && typed.len() > 6
        && crate::db::users::use_recovery_code(&state.pool, user_id, typed).await
    {
        log_event("recovery_code_used", &[("user", &email), ("ip", &ip)]);
        accepted = true;
    }

    if !accepted {
        failed_logins(|f| f.record(&email, Instant::now()));
        log_event("login_code_failed", &[("email", &email), ("ip", &ip)]);
        return show_error(
            StatusCode::OK,
            "That code isn't right. Try the newest one from your app.",
        );
    }

    failed_logins(|f| f.clear(&email));
    let _ = session.remove::<i64>(PENDING_USER_ID).await;
    let _ = session.remove::<u64>(PENDING_SINCE).await;
    log_event(
        "login",
        &[("user", &email), ("ip", &ip), ("two_factor", "yes")],
    );
    sign_in(&session, user_id, version).await;
    Redirect::to("/admin").into_response()
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn accounts_slow_down_after_repeated_failures() {
        let mut failed = FailedLogins::default();
        let start = Instant::now();
        for i in 0..FAILURES_ALLOWED {
            assert!(!failed.must_wait("a@x.com", start), "attempt {i} allowed");
            failed.record("a@x.com", start);
        }
        // Sixth try right away: wait. Other accounts aren't affected.
        assert!(failed.must_wait("a@x.com", start + Duration::from_secs(5)));
        assert!(!failed.must_wait("b@x.com", start));
        // A minute later one more try is allowed.
        assert!(!failed.must_wait("a@x.com", start + SLOWDOWN));
        // After the window, everything is forgotten.
        failed.record("a@x.com", start + SLOWDOWN);
        assert!(!failed.must_wait("a@x.com", start + FAILURE_WINDOW + SLOWDOWN * 2));
        // A successful sign-in clears the record.
        failed.record("c@x.com", start);
        failed.clear("c@x.com");
        assert!(!failed.0.contains_key("c@x.com"));
    }
}
