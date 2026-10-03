//! First-run setup: create the admin account and default settings.
//!
//! Like WordPress, the first visit to a new site opens a setup page in the
//! browser. Scripted installs can skip it with `BLOOGLA_ADMIN_EMAIL` /
//! `BLOOGLA_ADMIN_PASSWORD` (and optional `BLOOGLA_BLOG_NAME`).

use crate::app::security::{hash_password, missing_password_rules, PasswordRule, PASSWORD_RULES};
use crate::app::state::AppState;
use crate::i18n::Lang;

use askama::Template;
use axum::extract::{Query, State};
use axum::http::{header, HeaderMap};
use axum::response::{IntoResponse, Redirect, Response};
use axum_extra::extract::Form;
use serde::Deserialize;
use sqlx::SqlitePool;
use std::sync::atomic::Ordering;
use tower_sessions::Session;

#[derive(Template)]
#[template(path = "setup.html")]
pub struct SetupTemplate {
    pub lang: Lang,
    /// Choices for the language picker.
    pub languages: Vec<Lang>,
    pub blog_name: String,
    pub name: String,
    pub email: String,
    pub error: Option<String>,
    /// The password checklist.
    pub password_rules: &'static [PasswordRule],
    /// What the submitted password still needs (translated).
    pub password_missing: Vec<String>,
}

impl SetupTemplate {
    /// An empty form in `lang`.
    fn new(lang: Lang) -> Self {
        Self {
            lang,
            languages: Lang::all(),
            blog_name: String::new(),
            name: String::new(),
            email: String::new(),
            error: None,
            password_rules: PASSWORD_RULES,
            password_missing: Vec::new(),
        }
    }
}

/// True until the first admin account exists.
async fn needs_setup(pool: &SqlitePool) -> Result<bool, sqlx::Error> {
    let users: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM users")
        .fetch_one(pool)
        .await?;
    Ok(users == 0)
}

/// Create the first admin and default settings in one transaction.
/// Returns the new user's id.
async fn create_site(
    pool: &SqlitePool,
    blog_name: &str,
    name: &str,
    email: &str,
    password: &str,
) -> Result<i64, String> {
    let blog_name = blog_name.trim();
    let name = name.trim();
    let email = email.trim();
    if blog_name.is_empty() {
        return Err("Give your site a title.".into());
    }
    if !email.contains('@') {
        return Err("Enter a valid email address.".into());
    }
    // The browser form checks this first and lists what's missing; this
    // catches `BLOOGLA_ADMIN_PASSWORD` and anything else.
    let missing = missing_password_rules(password);
    if !missing.is_empty() {
        let list: Vec<String> = missing
            .iter()
            .map(|rule| rule.text.to_lowercase())
            .collect();
        return Err(format!("The password still needs: {}.", list.join(", ")));
    }

    let password_hash = hash_password(password)?;

    let db_error = |e: sqlx::Error| format!("Database error: {e}");
    let mut tx = pool.begin().await.map_err(db_error)?;

    // Re-check inside the transaction so two concurrent setups cannot both win.
    let users: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM users")
        .fetch_one(&mut *tx)
        .await
        .map_err(db_error)?;
    if users > 0 {
        return Err("This site has already been set up.".into());
    }

    let user_id = sqlx::query(
        "INSERT INTO users (name, email, password_hash, role) VALUES (?, ?, ?, 'admin')",
    )
    .bind(name)
    .bind(email)
    .bind(&password_hash)
    .execute(&mut *tx)
    .await
    .map_err(db_error)?
    .last_insert_rowid();
    let publisher_name = if name.is_empty() { blog_name } else { name };

    let default_settings = [
        ("blog_name", blog_name),
        (
            "blog_description",
            "A lightweight, high-performance blog built with Bloogla.",
        ),
        ("blog_keywords", "blog"),
        ("publisher_type", "Person"),
        ("publisher_name", publisher_name),
        ("active_theme", "default"),
    ];
    for (key, value) in default_settings {
        sqlx::query(
            "INSERT INTO settings (key, value) VALUES (?, ?)
             ON CONFLICT(key) DO UPDATE SET value = excluded.value",
        )
        .bind(key)
        .bind(value)
        .execute(&mut *tx)
        .await
        .map_err(db_error)?;
    }

    tx.commit().await.map_err(db_error)?;
    crate::db::settings::invalidate();
    Ok(user_id)
}

/// Prepare first-run setup.
///
/// With `BLOOGLA_ADMIN_EMAIL` and `BLOOGLA_ADMIN_PASSWORD` set, the site is
/// created right away (for scripted installs). Otherwise returns `true` and the
/// browser setup page at `/setup` stays open until someone completes it.
pub async fn prepare(
    pool: &SqlitePool,
    base_url: &str,
) -> Result<bool, Box<dyn std::error::Error>> {
    if !needs_setup(pool).await? {
        return Ok(false);
    }

    if let (Ok(email), Ok(password)) = (
        std::env::var("BLOOGLA_ADMIN_EMAIL"),
        std::env::var("BLOOGLA_ADMIN_PASSWORD"),
    ) {
        let blog_name = std::env::var("BLOOGLA_BLOG_NAME").unwrap_or_else(|_| "My Blog".into());
        let name = std::env::var("BLOOGLA_ADMIN_NAME").unwrap_or_default();
        create_site(pool, &blog_name, &name, &email, &password).await?;
        crate::app::security::log_event(
            "setup_completed",
            &[
                ("user", email.trim()),
                ("from", "BLOOGLA_ADMIN_* variables"),
            ],
        );
        return Ok(false);
    }

    tracing::info!("Almost done. Finish setup in your browser: {base_url}");
    Ok(true)
}

#[derive(Deserialize)]
pub struct SetupQuery {
    lang: Option<String>,
}

#[derive(Deserialize)]
pub struct SetupForm {
    #[serde(default)]
    language: String,
    blog_name: String,
    #[serde(default)]
    name: String,
    email: String,
    password: String,
    confirm_password: String,
}

/// GET /setup -> Browser setup form.
pub async fn page(
    State(state): State<AppState>,
    Query(query): Query<SetupQuery>,
    headers: HeaderMap,
) -> Response {
    if !state.setup_pending.load(Ordering::Acquire) {
        return Redirect::to("/admin").into_response();
    }
    // The picker's choice, else the browser's preferred language.
    let lang = query
        .lang
        .as_deref()
        .and_then(Lang::parse)
        .unwrap_or_else(|| {
            Lang::from_accept_language(
                headers
                    .get(header::ACCEPT_LANGUAGE)
                    .and_then(|v| v.to_str().ok())
                    .unwrap_or_default(),
            )
        });
    SetupTemplate::new(lang).into_response()
}

/// POST /setup -> Create the site and admin account, then sign in.
pub async fn submit(
    State(state): State<AppState>,
    session: Session,
    Form(form): Form<SetupForm>,
) -> Response {
    if !state.setup_pending.load(Ordering::Acquire) {
        return Redirect::to("/admin").into_response();
    }

    let lang = Lang::parse(&form.language).unwrap_or_default();
    // Show the form again with what was typed (except passwords) and a message.
    let refill = || SetupTemplate {
        blog_name: form.blog_name.clone(),
        name: form.name.clone(),
        email: form.email.clone(),
        ..SetupTemplate::new(lang)
    };
    let render_error = |message: &str| {
        SetupTemplate {
            error: Some(lang.t_owned(message)),
            ..refill()
        }
        .into_response()
    };

    let missing = missing_password_rules(&form.password);
    if !missing.is_empty() {
        return SetupTemplate {
            password_missing: missing.iter().map(|r| lang.t_owned(r.text)).collect(),
            ..refill()
        }
        .into_response();
    }
    if form.password != form.confirm_password {
        return render_error("The passwords don't match.");
    }
    let user_id = match create_site(
        &state.pool,
        &form.blog_name,
        &form.name,
        &form.email,
        &form.password,
    )
    .await
    {
        Ok(id) => id,
        Err(e) => return render_error(&e),
    };

    let _ = crate::db::settings::save(&state.pool, &[("language", lang.code().to_string())]).await;
    let _ = sqlx::query("UPDATE users SET language = ? WHERE id = ?")
        .bind(lang.code())
        .bind(user_id)
        .execute(&state.pool)
        .await;

    state.setup_pending.store(false, Ordering::Release);
    crate::app::security::log_event("setup_completed", &[("user", form.email.trim())]);
    // A brand-new account starts at session version 0.
    super::auth::sign_in(&session, user_id, 0).await;

    Redirect::to("/admin?welcome=1").into_response()
}
