//! First-run setup: create the admin account and default settings.
//!
//! Like WordPress, the first visit to a new site opens a setup page in the
//! browser. Scripted installs can skip it with `BLOOGLA_ADMIN_EMAIL` /
//! `BLOOGLA_ADMIN_PASSWORD` (and optional `BLOOGLA_BLOG_NAME`).

use crate::models::AppState;
use crate::templates::SetupTemplate;

use argon2::password_hash::{PasswordHasher, SaltString};
use argon2::Argon2;
use axum::extract::{Request, State};
use axum::middleware::Next;
use axum::response::{IntoResponse, Redirect, Response};
use axum_extra::extract::Form;
use console::Style;
use rand::thread_rng;
use serde::Deserialize;
use sqlx::SqlitePool;
use std::sync::atomic::Ordering;
use tower_sessions::Session;

pub const MIN_PASSWORD_LEN: usize = 8;

/// True until the first admin account exists.
pub async fn needs_setup(pool: &SqlitePool) -> Result<bool, sqlx::Error> {
    let users: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM users")
        .fetch_one(pool)
        .await?;
    Ok(users == 0)
}

/// Create the admin user and default settings in one transaction.
pub async fn create_site(
    pool: &SqlitePool,
    blog_name: &str,
    email: &str,
    password: &str,
) -> Result<(), String> {
    let blog_name = blog_name.trim();
    let email = email.trim();
    if blog_name.is_empty() {
        return Err("Give your site a title.".into());
    }
    if !email.contains('@') {
        return Err("Enter a valid email address.".into());
    }
    if password.chars().count() < MIN_PASSWORD_LEN {
        return Err(format!(
            "Use a password with at least {MIN_PASSWORD_LEN} characters."
        ));
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

    sqlx::query("INSERT INTO users (email, password_hash) VALUES (?, ?)")
        .bind(email)
        .bind(&password_hash)
        .execute(&mut *tx)
        .await
        .map_err(db_error)?;

    let default_settings = [
        ("blog_name", blog_name),
        (
            "blog_description",
            "A lightweight, high-performance blog built with Bloogla.",
        ),
        ("blog_keywords", "blog"),
        ("publisher_type", "Person"),
        ("publisher_name", blog_name),
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
    crate::utils::invalidate_settings();
    Ok(())
}

pub fn hash_password(password: &str) -> Result<String, String> {
    let salt = SaltString::generate(&mut thread_rng());
    Argon2::default()
        .hash_password(password.as_bytes(), &salt)
        .map(|h| h.to_string())
        .map_err(|e| format!("Password hashing failed: {e}"))
}

/// Prepare first-run setup.
///
/// With `BLOOGLA_ADMIN_EMAIL` and `BLOOGLA_ADMIN_PASSWORD` set, the site is
/// created right away (for scripted installs). Otherwise returns `true` and the
/// browser setup page at `/setup` stays open until someone completes it.
pub async fn prepare_first_run(
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
        create_site(pool, &blog_name, &email, &password).await?;
        println!("Admin account created from BLOOGLA_ADMIN_* environment variables.");
        return Ok(false);
    }

    let cyan = Style::new().cyan().bold();
    println!(
        "{} {base_url}",
        cyan.apply_to("Almost done. Finish setup in your browser:")
    );
    Ok(true)
}

/// While setup is pending, send every page to `/setup`.
pub async fn setup_guard(State(state): State<AppState>, req: Request, next: Next) -> Response {
    let path = req.uri().path();
    let exempt = path.starts_with("/setup") || path.starts_with("/static/") || path == "/health";

    if !exempt && state.setup_pending.load(Ordering::Acquire) {
        return Redirect::to("/setup").into_response();
    }
    next.run(req).await
}

#[derive(Deserialize)]
pub struct SetupForm {
    blog_name: String,
    email: String,
    password: String,
    confirm_password: String,
}

/// GET /setup -> Browser setup form.
pub async fn setup_page(State(state): State<AppState>) -> Response {
    if !state.setup_pending.load(Ordering::Acquire) {
        return Redirect::to("/admin").into_response();
    }
    SetupTemplate {
        blog_name: String::new(),
        email: String::new(),
        error: None,
    }
    .into_response()
}

/// POST /setup -> Create the site and admin account, then sign in.
pub async fn handle_setup(
    State(state): State<AppState>,
    session: Session,
    Form(form): Form<SetupForm>,
) -> Response {
    if !state.setup_pending.load(Ordering::Acquire) {
        return Redirect::to("/admin").into_response();
    }

    let render_error = |message: &str| {
        SetupTemplate {
            blog_name: form.blog_name.clone(),
            email: form.email.clone(),
            error: Some(message.to_string()),
        }
        .into_response()
    };

    if form.password != form.confirm_password {
        return render_error("The passwords don't match.");
    }
    if let Err(e) = create_site(&state.pool, &form.blog_name, &form.email, &form.password).await {
        return render_error(&e);
    }

    state.setup_pending.store(false, Ordering::Release);
    if let Err(e) = session.cycle_id().await {
        eprintln!("Failed to cycle session id: {e}");
    }
    let _ = session.insert("admin_logged_in", true).await;

    Redirect::to("/admin?welcome=1").into_response()
}
