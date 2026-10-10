//! Your own account: name, email, language, password and two-factor login.

use super::{alert, saved_and_reload};
use crate::app::models::CurrentUser;
use crate::app::security::{
    from_hex, hash_password, hash_recovery_code, log_event, missing_password_rules,
    new_recovery_codes, to_hex, verify_password, PasswordRule, PASSWORD_RULES,
};
use crate::app::state::AppState;
use crate::db::{or_log, settings};
use crate::i18n::Lang;

use askama::Template;
use axum::extract::{Extension, State};
use axum::http::StatusCode;
use axum::response::{Html, IntoResponse, Redirect, Response};
use axum_extra::extract::Form;
use serde::Deserialize;
use tower_sessions::Session;

/// The signed-in person's own name, email and password.
#[derive(Template)]
#[template(path = "profile.html")]
pub struct ProfileTemplate {
    pub blog_name: String,
    pub me: CurrentUser,
    /// The person's own language choice (`""` = the site's).
    pub language: String,
    pub languages: Vec<Lang>,
    /// The password checklist.
    pub password_rules: &'static [PasswordRule],
    /// Whether two-factor login is on, and how many recovery codes are unused.
    pub two_factor: bool,
    pub recovery_codes_left: i64,
}

/// Turning on two-factor login (`codes` empty), or the recovery codes to save
/// (shown once, right after turning it on or making new ones).
#[derive(Template)]
#[template(path = "two_factor.html")]
pub struct TwoFactorTemplate {
    pub blog_name: String,
    pub me: CurrentUser,
    /// QR code (SVG) with the setup link for the app.
    pub qr_svg: String,
    /// The key in Base32, for typing into the app instead of scanning.
    pub secret_text: String,
    pub error: Option<String>,
    pub codes: Vec<String>,
}

/// Session key holding the new key while two-factor login is being set up.
const TWO_FACTOR_SETUP: &str = "two_factor_setup";

fn unix_now() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_secs())
}

#[derive(Deserialize)]
pub struct ProfileForm {
    name: String,
    email: String,
    /// `en`, `tr`, or empty for the site's language.
    #[serde(default)]
    language: String,
    /// Required only when changing the sign-in and recovery address.
    #[serde(default)]
    current_password: String,
}

#[derive(Deserialize)]
pub struct PasswordForm {
    current_password: String,
    new_password: String,
    confirm_password: String,
}

/// Every sensitive profile action shares the login budget. Reserving before
/// the async password check also covers concurrent guesses through different
/// forms; a successful confirmation releases only its own slot.
async fn confirm_password(
    state: &AppState,
    me: &CurrentUser,
    password: String,
) -> Result<(), Response> {
    let Some(attempt) = super::auth::reserve_attempt(&me.email) else {
        return Err((
            StatusCode::TOO_MANY_REQUESTS,
            alert(
                me.lang,
                "error",
                "Too many failed sign-ins for this account. Wait a minute and try again.",
            ),
        )
            .into_response());
    };
    let stored: Option<String> = or_log(
        sqlx::query_scalar("SELECT password_hash FROM users WHERE id = ?")
            .bind(me.id)
            .fetch_optional(&state.pool)
            .await,
        "load password",
    );
    let valid = tokio::task::spawn_blocking(move || verify_password(&password, stored.as_deref()))
        .await
        .unwrap_or(false);
    super::auth::finish_password_check(&me.email, attempt, valid);
    if !valid {
        log_event("password_confirmation_failed", &[("user", &me.email)]);
        return Err((
            StatusCode::UNPROCESSABLE_ENTITY,
            alert(me.lang, "error", "Your current password isn't right."),
        )
            .into_response());
    }
    Ok(())
}

/// GET /admin/profile -> Your own name, email and password.
pub async fn page(
    State(state): State<AppState>,
    Extension(me): Extension<CurrentUser>,
) -> impl IntoResponse {
    let language: String = or_log(
        sqlx::query_scalar("SELECT language FROM users WHERE id = ?")
            .bind(me.id)
            .fetch_one(&state.pool)
            .await,
        "load language",
    );
    ProfileTemplate {
        blog_name: settings::load(&state.pool).await.blog_name.clone(),
        language,
        languages: Lang::all(),
        password_rules: PASSWORD_RULES,
        two_factor: crate::db::users::two_factor(&state.pool, me.id)
            .await
            .is_some(),
        recovery_codes_left: crate::db::users::recovery_codes_left(&state.pool, me.id).await,
        me,
    }
}

/// POST /admin/profile -> Update your name and email.
pub async fn update(
    State(state): State<AppState>,
    Extension(me): Extension<CurrentUser>,
    session: Session,
    Form(form): Form<ProfileForm>,
) -> Response {
    let email = form.email.trim();
    if !email.contains('@') {
        return alert(me.lang, "error", "Enter a valid email address.");
    }
    let taken: Option<i64> = or_log(
        sqlx::query_scalar("SELECT id FROM users WHERE email = ? COLLATE NOCASE AND id != ?")
            .bind(email)
            .bind(me.id)
            .fetch_optional(&state.pool)
            .await,
        "check email",
    );
    if taken.is_some() {
        return alert(me.lang, "error", "Someone else already uses this email.");
    }

    let email_changed = email != me.email;
    if email_changed {
        if let Err(response) = confirm_password(&state, &me, form.current_password).await {
            return response;
        }
    }

    let language = Lang::parse(&form.language).map_or("", Lang::code);
    let previous_language: String = or_log(
        sqlx::query_scalar("SELECT language FROM users WHERE id = ?")
            .bind(me.id)
            .fetch_one(&state.pool)
            .await,
        "load language",
    );
    match crate::db::users::update_profile(&state.pool, &me, form.name.trim(), email, language)
        .await
    {
        Ok(Some(version)) => {
            if email_changed {
                log_event(
                    "email_changed",
                    &[("user", &me.email), ("new_email", email)],
                );
                super::auth::sign_in(&session, me.id, version).await;
            }
            // Reload after an address change too, to clear the confirmation
            // field and show the saved address and session everywhere.
            if language != previous_language || email_changed {
                let site_language = settings::load(&state.pool).await.language;
                saved_and_reload(Lang::parse(language).unwrap_or(site_language))
            } else {
                alert(me.lang, "success", "Saved.")
            }
        }
        Ok(None) => {
            let _ = session.flush().await;
            StatusCode::UNAUTHORIZED.into_response()
        }
        Err(e) => {
            tracing::error!("Failed to update profile: {e}");
            alert(
                me.lang,
                "error",
                "Couldn't save your changes. Please try again.",
            )
        }
    }
}

/// POST /admin/profile/password -> Change your own password.
pub async fn update_password(
    State(state): State<AppState>,
    Extension(me): Extension<CurrentUser>,
    session: Session,
    Form(form): Form<PasswordForm>,
) -> Response {
    let missing = missing_password_rules(&form.new_password);
    if !missing.is_empty() {
        let items: String = missing
            .iter()
            .map(|rule| format!("<li>{}</li>", me.lang.t_owned(rule.text)))
            .collect();
        return Html(format!(
            r#"<div class="alert alert-error">{}<ul class="password-missing">{items}</ul></div>"#,
            me.t("Your password still needs:")
        ))
        .into_response();
    }
    if form.new_password != form.confirm_password {
        return alert(me.lang, "error", "The new passwords don't match.");
    }

    if let Err(response) = confirm_password(&state, &me, form.current_password).await {
        return response;
    }

    let Ok(Ok(hash)) = tokio::task::spawn_blocking(move || hash_password(&form.new_password)).await
    else {
        return alert(
            me.lang,
            "error",
            "Couldn't change your password. Please try again.",
        );
    };
    match crate::db::users::change_password(&state.pool, me.id, &hash, Some(me.session_version))
        .await
    {
        Ok(Some(new_version)) => {
            log_event("password_changed", &[("user", &me.email)]);
            // Every other device is now signed out; this one stays signed in.
            crate::handlers::admin::auth::sign_in(&session, me.id, new_version).await;
            alert(
                me.lang,
                "success",
                "Password updated. Other devices have been signed out.",
            )
        }
        Ok(None) => {
            let _ = session.flush().await;
            Redirect::to("/admin/login").into_response()
        }
        Err(e) => {
            tracing::error!("Failed to update password: {e}");
            alert(
                me.lang,
                "error",
                "Couldn't change your password. Please try again.",
            )
        }
    }
}

// ---- Two-factor login (optional, recommended) ----

async fn setup_page(
    state: &AppState,
    me: CurrentUser,
    secret: &[u8],
    error: Option<String>,
) -> Response {
    let blog_name = settings::load(&state.pool).await.blog_name.clone();
    let uri = crate::app::totp::setup_uri(secret, &blog_name, &me.email);
    TwoFactorTemplate {
        blog_name,
        me,
        qr_svg: crate::app::totp::qr_svg(&uri),
        secret_text: crate::app::totp::display_secret(secret),
        error,
        codes: Vec::new(),
    }
    .into_response()
}

/// Make new recovery codes for `me`, store their hashes and show them once.
async fn show_new_codes(
    state: &AppState,
    mut me: CurrentUser,
    session: &Session,
    secret: Option<(&[u8], u64)>,
) -> Response {
    let codes = new_recovery_codes();
    let hashes: Vec<String> = codes.iter().map(|c| hash_recovery_code(c)).collect();
    let saved = match secret {
        Some((secret, step)) => {
            crate::db::users::enable_two_factor(
                &state.pool,
                me.id,
                secret,
                step,
                &hashes,
                me.session_version,
            )
            .await
        }
        None => {
            crate::db::users::set_recovery_codes(&state.pool, me.id, &hashes, me.session_version)
                .await
        }
    };
    match saved {
        Ok(Some(version)) => {
            // Revoke existing password-only sessions when two-factor login is
            // enabled, and keep only the session that confirmed this change.
            crate::handlers::admin::auth::sign_in(session, me.id, version).await;
            me.session_version = version;
        }
        Ok(None) => {
            let _ = session.flush().await;
            return Redirect::to("/admin/login").into_response();
        }
        Err(e) => {
            tracing::error!("Failed to save two-factor login: {e}");
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Html(me.t("Couldn't save your changes. Please try again.")),
            )
                .into_response();
        }
    }
    TwoFactorTemplate {
        blog_name: settings::load(&state.pool).await.blog_name.clone(),
        me,
        qr_svg: String::new(),
        secret_text: String::new(),
        error: None,
        codes,
    }
    .into_response()
}

/// GET /admin/profile/two-factor -> Scan a QR code to turn on two-factor login.
pub async fn two_factor_page(
    State(state): State<AppState>,
    Extension(me): Extension<CurrentUser>,
    session: Session,
) -> Response {
    if crate::db::users::two_factor(&state.pool, me.id)
        .await
        .is_some()
    {
        return Redirect::to("/admin/profile#two-factor").into_response();
    }
    // A new key each time the page opens; it's saved only once confirmed.
    let secret = crate::app::totp::new_secret();
    if let Err(e) = session.insert(TWO_FACTOR_SETUP, to_hex(&secret)).await {
        tracing::error!("Failed to save session: {e}");
    }
    setup_page(&state, me, &secret, None).await
}

#[derive(Deserialize)]
pub struct CodeForm {
    code: String,
    #[serde(default)]
    password: String,
}

/// POST /admin/profile/two-factor -> Confirm with a code from the app and turn it on.
pub async fn enable_two_factor(
    State(state): State<AppState>,
    Extension(me): Extension<CurrentUser>,
    session: Session,
    Form(form): Form<CodeForm>,
) -> Response {
    let hex: Option<String> = session.get(TWO_FACTOR_SETUP).await.ok().flatten();
    let secret = hex.as_deref().and_then(from_hex);
    let Some(secret) = secret else {
        return Redirect::to("/admin/profile/two-factor").into_response();
    };
    if let Err(response) = confirm_password(&state, &me, form.password).await {
        let status = response.status();
        let message = if status == StatusCode::TOO_MANY_REQUESTS {
            "Too many failed sign-ins for this account. Wait a minute and try again."
        } else {
            "Your current password isn't right."
        };
        let error = me.t(message).to_string();
        return (status, setup_page(&state, me, &secret, Some(error)).await).into_response();
    }
    let Some(step) = crate::app::totp::verify(&secret, &form.code, unix_now(), 0) else {
        let error = me
            .t("That code isn't right. Try the newest one from your app.")
            .to_string();
        return setup_page(&state, me, &secret, Some(error)).await;
    };
    let _ = session.remove::<String>(TWO_FACTOR_SETUP).await;
    log_event("two_factor_enabled", &[("user", &me.email)]);
    show_new_codes(&state, me, &session, Some((&secret, step))).await
}

/// POST /admin/profile/two-factor/recovery -> Replace the recovery codes.
pub async fn new_recovery_codes_page(
    State(state): State<AppState>,
    Extension(me): Extension<CurrentUser>,
    session: Session,
    Form(form): Form<ConfirmPasswordForm>,
) -> Response {
    if crate::db::users::two_factor(&state.pool, me.id)
        .await
        .is_none()
    {
        return Redirect::to("/admin/profile#two-factor").into_response();
    }
    if let Err(response) = confirm_password(&state, &me, form.password).await {
        return response;
    }
    log_event("recovery_codes_replaced", &[("user", &me.email)]);
    show_new_codes(&state, me, &session, None).await
}

#[derive(Deserialize)]
pub struct ConfirmPasswordForm {
    password: String,
}

/// POST /admin/profile/two-factor/off -> Turn it off (asks for the password).
pub async fn disable_two_factor(
    State(state): State<AppState>,
    Extension(me): Extension<CurrentUser>,
    Form(form): Form<ConfirmPasswordForm>,
) -> Response {
    if let Err(response) = confirm_password(&state, &me, form.password).await {
        return response;
    }
    match crate::db::users::disable_two_factor_if_current(
        &state.pool,
        me.id,
        Some(me.session_version),
    )
    .await
    {
        Ok(true) => {
            log_event("two_factor_disabled", &[("user", &me.email)]);
            saved_and_reload(me.lang)
        }
        Ok(false) => StatusCode::UNAUTHORIZED.into_response(),
        Err(e) => {
            tracing::error!("Failed to turn off two-factor login: {e}");
            alert(
                me.lang,
                "error",
                "Couldn't save your changes. Please try again.",
            )
        }
    }
}
