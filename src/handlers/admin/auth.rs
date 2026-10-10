//! Signing in and out of the admin panel.

use crate::app::models::{SESSION_USER_ID, SESSION_VERSION};
use crate::app::security::log_event;
use crate::app::state::AppState;
use crate::db::settings;
use crate::i18n::Lang;

use askama::Template;
use axum::extract::{ConnectInfo, Form, Query, State};
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
    /// Good news to show above the form (e.g. after a password reset).
    pub notice: Option<String>,
}

#[derive(Deserialize)]
pub struct LoginQuery {
    /// Set after choosing a new password from an emailed link.
    reset: Option<String>,
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
struct FailedLogins {
    accounts: HashMap<String, VecDeque<LoginAttempt>>,
    next_id: u64,
}

struct LoginAttempt {
    id: u64,
    started: Instant,
    pending: bool,
}

impl FailedLogins {
    fn prune(&mut self, now: Instant) {
        self.accounts.retain(|_, attempts| {
            attempts.retain(|a| now.duration_since(a.started) < FAILURE_WINDOW);
            !attempts.is_empty()
        });
    }

    /// Reserve a slot under the mutex before any async credential checks.
    /// In-flight requests count too, so a burst cannot bypass the limit.
    fn reserve(&mut self, email: &str, now: Instant) -> Option<u64> {
        self.prune(now);
        let attempts = self.accounts.entry(email.to_string()).or_default();
        if attempts.len() >= FAILURES_ALLOWED
            && attempts
                .back()
                .is_some_and(|a| now.duration_since(a.started) < SLOWDOWN)
        {
            return None;
        }
        self.next_id += 1;
        attempts.push_back(LoginAttempt {
            id: self.next_id,
            started: now,
            pending: true,
        });
        Some(self.next_id)
    }

    fn finish(&mut self, email: &str, id: u64, valid: bool, signed_in: bool) {
        if let Some(attempts) = self.accounts.get_mut(email) {
            if let Some(attempt) = attempts.iter_mut().find(|a| a.id == id) {
                attempt.pending = false;
            }
            // Checking a correct password before two-factor login only releases
            // its own slot. It must not clear previous failed code guesses.
            // A completed sign-in clears failures but preserves other requests
            // still checking credentials, including ones that will fail later.
            attempts.retain(|a| !(valid && a.id == id || signed_in && !a.pending));
        }
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

#[cfg(test)]
pub(crate) fn clear_test_attempts() {
    failed_logins(|f| f.accounts.clear());
}

/// Profile actions that ask for the password share the account's budget.
pub(super) fn reserve_attempt(email: &str) -> Option<u64> {
    failed_logins(|f| f.reserve(&email.to_lowercase(), Instant::now()))
}

pub(super) fn finish_password_check(email: &str, id: u64, valid: bool) {
    failed_logins(|f| f.finish(&email.to_lowercase(), id, valid, false));
}

/// GET /admin/login -> Sign-in form.
pub async fn login_page(
    State(state): State<AppState>,
    Query(query): Query<LoginQuery>,
) -> impl IntoResponse {
    let lang = settings::load(&state.pool).await.language;
    LoginTemplate {
        lang,
        error: None,
        notice: query.reset.map(|_| {
            lang.t("Your new password is saved. Sign in with it.")
                .to_string()
        }),
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

    let attempt = failed_logins(|f| f.reserve(&email, Instant::now()));
    let Some(attempt) = attempt else {
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
                notice: None,
            },
        )
            .into_response();
    };
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
            failed_logins(|f| f.finish(&email, attempt, true, false));
            // The password was right; now ask for the code from the app.
            let waiting = async {
                session.cycle_id().await?;
                session.insert(PENDING_USER_ID, user_id).await?;
                session.insert(PENDING_VERSION, version).await?;
                session.insert(PENDING_SINCE, unix_now()).await
            }
            .await;
            if let Err(e) = waiting {
                tracing::error!("Failed to save session: {e}");
            }
            return Redirect::to("/admin/login/code").into_response();
        }
        failed_logins(|f| f.finish(&email, attempt, true, true));
        log_event("login", &[("user", &email), ("ip", &ip)]);
        sign_in(&session, user_id, version).await;
        return Redirect::to("/admin").into_response();
    }
    failed_logins(|f| f.finish(&email, attempt, false, false));
    log_event("login_failed", &[("email", &email), ("ip", &ip)]);

    let lang = settings::load(&state.pool).await.language;
    LoginTemplate {
        lang,
        error: Some(lang.t("Invalid email or password.").into()),
        notice: None,
    }
    .into_response()
}

// ---- Second step for people with two-factor login ----

/// Session keys while the password was right but the code isn't entered yet.
const PENDING_USER_ID: &str = "pending_user_id";
const PENDING_VERSION: &str = "pending_version";
const PENDING_SINCE: &str = "pending_since";
/// How long the code page stays valid after the password.
const PENDING_SECONDS: u64 = 5 * 60;

fn unix_now() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_secs())
}

/// The account waiting for its code, if the password was entered recently.
async fn pending_user(state: &AppState, session: &Session) -> Option<(i64, i64)> {
    let since: u64 = session.get(PENDING_SINCE).await.ok().flatten()?;
    if unix_now().saturating_sub(since) > PENDING_SECONDS {
        return None;
    }
    let id: i64 = session.get(PENDING_USER_ID).await.ok().flatten()?;
    let version: i64 = session.get(PENDING_VERSION).await.ok().flatten()?;
    let current: Option<i64> = sqlx::query_scalar("SELECT session_version FROM users WHERE id = ?")
        .bind(id)
        .fetch_optional(&state.pool)
        .await
        .ok()?;
    // A password change revokes the password already checked in the first
    // step, as well as completed sign-ins.
    if current != Some(version) {
        clear_pending(session).await;
        return None;
    }
    Some((id, version))
}

async fn clear_pending(session: &Session) {
    let _ = session.remove::<i64>(PENDING_USER_ID).await;
    let _ = session.remove::<i64>(PENDING_VERSION).await;
    let _ = session.remove::<u64>(PENDING_SINCE).await;
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
    if pending_user(&state, &session).await.is_none() {
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
    let Some((user_id, version)) = pending_user(&state, &session).await else {
        return Redirect::to("/admin/login").into_response();
    };
    let ip = crate::handlers::client_ip(&state, peer, &headers).to_string();
    let lang = settings::load(&state.pool).await.language;
    let user: Option<String> = sqlx::query_scalar("SELECT email FROM users WHERE id = ?")
        .bind(user_id)
        .fetch_optional(&state.pool)
        .await
        .unwrap_or(None);
    let Some(email) = user else {
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

    let attempt = failed_logins(|f| f.reserve(&email, Instant::now()));
    let Some(attempt) = attempt else {
        log_event("login_throttled", &[("email", &email), ("ip", &ip)]);
        return show_error(
            StatusCode::TOO_MANY_REQUESTS,
            "Too many failed sign-ins for this account. Wait a minute and try again.",
        );
    };

    let typed = form.code.trim();
    let mut accepted = false;
    if let Some((secret, last_step)) = crate::db::users::two_factor(&state.pool, user_id).await {
        if let Some(step) = crate::app::totp::verify(&secret, typed, unix_now(), last_step) {
            accepted = crate::db::users::set_two_factor_step(&state.pool, user_id, step).await;
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
        failed_logins(|f| f.finish(&email, attempt, false, false));
        log_event("login_code_failed", &[("email", &email), ("ip", &ip)]);
        return show_error(
            StatusCode::OK,
            "That code isn't right. Try the newest one from your app.",
        );
    }

    failed_logins(|f| f.finish(&email, attempt, true, true));
    clear_pending(&session).await;
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
        for _ in 0..FAILURES_ALLOWED {
            let id = failed.reserve("a@x.com", start).unwrap();
            failed.finish("a@x.com", id, false, false);
        }
        assert!(failed
            .reserve("a@x.com", start + Duration::from_secs(5))
            .is_none());
        assert!(failed.reserve("b@x.com", start).is_some());
        assert!(failed.reserve("a@x.com", start + SLOWDOWN).is_some());
        assert!(failed
            .reserve("a@x.com", start + SLOWDOWN + Duration::from_secs(1))
            .is_none());
        assert!(failed
            .reserve("a@x.com", start + FAILURE_WINDOW + SLOWDOWN * 2)
            .is_some());
    }

    #[test]
    fn pending_attempts_survive_a_successful_sign_in() {
        let mut failed = FailedLogins::default();
        let now = Instant::now();
        let good = failed.reserve("a@x.com", now).unwrap();
        for _ in 1..FAILURES_ALLOWED {
            failed.reserve("a@x.com", now).unwrap();
        }
        assert!(failed.reserve("a@x.com", now).is_none());
        failed.finish("a@x.com", good, true, true);
        // Only the successful request's slot was released.
        let id = failed.reserve("a@x.com", now).unwrap();
        assert!(failed.reserve("a@x.com", now).is_none());
        failed.finish("a@x.com", id, true, false);
        assert_eq!(failed.accounts["a@x.com"].len(), 4);
    }
}
