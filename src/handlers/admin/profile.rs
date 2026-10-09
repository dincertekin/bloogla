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
}

#[derive(Deserialize)]
pub struct PasswordForm {
    current_password: String,
    new_password: String,
    confirm_password: String,
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

    let language = Lang::parse(&form.language).map_or("", Lang::code);
    let previous_language: String = or_log(
        sqlx::query_scalar("SELECT language FROM users WHERE id = ?")
            .bind(me.id)
            .fetch_one(&state.pool)
            .await,
        "load language",
    );
    match sqlx::query("UPDATE users SET name = ?, email = ?, language = ? WHERE id = ?")
        .bind(form.name.trim())
        .bind(email)
        .bind(language)
        .bind(me.id)
        .execute(&state.pool)
        .await
    {
        // A new language applies to the whole page, so reload it.
        Ok(_) if language != previous_language => {
            let site_language = settings::load(&state.pool).await.language;
            saved_and_reload(Lang::parse(language).unwrap_or(site_language))
        }
        Ok(_) => alert(me.lang, "success", "Saved."),
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

    let stored: Option<String> = or_log(
        sqlx::query_scalar("SELECT password_hash FROM users WHERE id = ?")
            .bind(me.id)
            .fetch_optional(&state.pool)
            .await,
        "load password",
    );
    // Password hashing is slow on purpose, so it runs off the async threads.
    let current_password = form.current_password.clone();
    let current_ok =
        tokio::task::spawn_blocking(move || verify_password(&current_password, stored.as_deref()))
            .await
            .unwrap_or(false);
    if !current_ok {
        log_event("password_change_failed", &[("user", &me.email)]);
        return alert(me.lang, "error", "Your current password isn't right.");
    }

    let Ok(hash) = hash_password(&form.new_password) else {
        return alert(
            me.lang,
            "error",
            "Couldn't change your password. Please try again.",
        );
    };
    match crate::db::users::set_password(&state.pool, me.id, &hash).await {
        Ok(new_version) => {
            log_event("password_changed", &[("user", &me.email)]);
            // Every other device is now signed out; this one stays signed in.
            crate::handlers::admin::auth::sign_in(&session, me.id, new_version).await;
            alert(
                me.lang,
                "success",
                "Password updated. Other devices have been signed out.",
            )
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
    me: CurrentUser,
    secret: Option<(&[u8], u64)>,
) -> Response {
    let codes = new_recovery_codes();
    let hashes: Vec<String> = codes.iter().map(|c| hash_recovery_code(c)).collect();
    let saved = match secret {
        Some((secret, step)) => {
            crate::db::users::enable_two_factor(&state.pool, me.id, secret, step, &hashes).await
        }
        None => crate::db::users::set_recovery_codes(&state.pool, me.id, &hashes).await,
    };
    if let Err(e) = saved {
        tracing::error!("Failed to save two-factor login: {e}");
        return (
            StatusCode::INTERNAL_SERVER_ERROR,
            Html(me.t("Couldn't save your changes. Please try again.")),
        )
            .into_response();
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
    let Some(step) = crate::app::totp::verify(&secret, &form.code, unix_now(), 0) else {
        let error = me
            .t("That code isn't right. Try the newest one from your app.")
            .to_string();
        return setup_page(&state, me, &secret, Some(error)).await;
    };
    let _ = session.remove::<String>(TWO_FACTOR_SETUP).await;
    log_event("two_factor_enabled", &[("user", &me.email)]);
    show_new_codes(&state, me, Some((&secret, step))).await
}

/// POST /admin/profile/two-factor/recovery -> Replace the recovery codes.
pub async fn new_recovery_codes_page(
    State(state): State<AppState>,
    Extension(me): Extension<CurrentUser>,
) -> Response {
    if crate::db::users::two_factor(&state.pool, me.id)
        .await
        .is_none()
    {
        return Redirect::to("/admin/profile#two-factor").into_response();
    }
    log_event("recovery_codes_replaced", &[("user", &me.email)]);
    show_new_codes(&state, me, None).await
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
    let stored: Option<String> = or_log(
        sqlx::query_scalar("SELECT password_hash FROM users WHERE id = ?")
            .bind(me.id)
            .fetch_optional(&state.pool)
            .await,
        "load password",
    );
    let password = form.password;
    let ok = tokio::task::spawn_blocking(move || verify_password(&password, stored.as_deref()))
        .await
        .unwrap_or(false);
    if !ok {
        return alert(me.lang, "error", "Your current password isn't right.");
    }
    match crate::db::users::disable_two_factor(&state.pool, me.id).await {
        Ok(_) => {
            log_event("two_factor_disabled", &[("user", &me.email)]);
            saved_and_reload(me.lang)
        }
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
