mod config;
mod db;
mod handlers;
mod models;
mod tags;
mod templates;
mod utils;

use argon2::password_hash::{PasswordHasher, SaltString};
use argon2::Argon2;
use axum::http::{header, HeaderValue};
use axum::{
    routing::{delete, get, post},
    Router,
};
use config::Config;
use console::Style;
use dialoguer::{Input, Password};
use models::AppState;
use rand::thread_rng;
use sqlx::SqlitePool;
use std::fs;
use std::net::SocketAddr;
use std::path::Path;
use std::sync::Arc;
use tower_governor::key_extractor::SmartIpKeyExtractor;
use tower_governor::{governor::GovernorConfigBuilder, GovernorLayer};
use tower_http::{
    compression::CompressionLayer, services::ServeDir, set_header::SetResponseHeaderLayer,
};
use tower_sessions::{MemoryStore, SessionManagerLayer};

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let cyan = Style::new().cyan().bold();
    let green = Style::new().green().bold();

    println!("{}", cyan.apply_to("\n🚀 Welcome to Bloogla\n"));

    // 1. Ensure local data directory exists before connecting to SQLite
    if !Path::new("data").exists() {
        fs::create_dir_all("data")?;
    }

    let config = Config::default();
    let db_file_exists = Path::new("data/bloogla.db").exists();

    // Connect to SQLite (creates empty bloogla.db file if it doesn't exist)
    let pool = db::init_db(&config.database_url).await?;
    println!("{}", green.apply_to("✔ Database connection established!"));

    // If bloogla.db did not exist on disk prior to boot, run the CLI setup wizard
    if !db_file_exists {
        run_cli_wizard(&pool).await?;
    }

    let state = AppState {
        pool,
        config: config.clone(),
    };

    // Session Layer
    let session_store = MemoryStore::default();
    let session_layer = SessionManagerLayer::new(session_store).with_secure(config.production);

    // Rate Limiter for Login Endpoint
    let governor_conf = Arc::new(
        GovernorConfigBuilder::default()
            .per_second(12)
            .burst_size(5)
            .key_extractor(SmartIpKeyExtractor)
            .finish()
            .unwrap(),
    );

    // 1 Year Cache Layer for Static Files
    let static_cache_layer = SetResponseHeaderLayer::overriding(
        header::CACHE_CONTROL,
        HeaderValue::from_static("public, max-age=31536000, immutable"),
    );

    // Axum Web Server Router
    let app = Router::new()
        .nest(
            "/static",
            Router::new()
                .fallback_service(ServeDir::new("static"))
                .layer(static_cache_layer),
        )
        // Public Endpoints
        .route("/", get(handlers::public::home_page))
        .route("/search", get(handlers::public::search_article))
        .route("/article/:slug", get(handlers::public::show_article))
        .route("/tag/:slug", get(handlers::public::tag_page))
        .route("/health", get(handlers::public::health_check))
        .route("/rss.xml", get(handlers::public::rss_feed))
        .route("/sitemap.xml", get(handlers::public::sitemap_xml))
        // Authentication Routes
        .route("/admin/login", get(handlers::auth::login_page))
        .route(
            "/admin/login",
            post(handlers::auth::handle_login).layer(GovernorLayer {
                config: governor_conf,
            }),
        )
        .route("/admin/logout", get(handlers::auth::handle_logout))
        // Admin & Settings Management Routes
        .route("/admin", get(handlers::admin::admin_dashboard))
        .route("/admin/settings", get(handlers::admin::settings_page))
        .route(
            "/admin/settings/general",
            post(handlers::admin::update_general_settings),
        )
        .route(
            "/admin/settings/password",
            post(handlers::admin::update_password),
        )
        // Article CRUD Routes
        .route("/admin/articles", get(handlers::admin::list_articles))
        .route("/admin/articles/new", post(handlers::admin::create_article))
        .route(
            "/admin/articles/:id/edit",
            get(handlers::admin::edit_article_page).post(handlers::admin::edit_article),
        )
        .route(
            "/admin/articles/:id",
            delete(handlers::admin::delete_article),
        )
        // Tag CRUD Routes
        .route("/admin/tags", get(handlers::admin::tags_page))
        .route("/admin/tags/new", post(handlers::admin::create_tag))
        .route("/admin/tags/:id", delete(handlers::admin::delete_tag))
        // Global Security Headers
        .layer(SetResponseHeaderLayer::overriding(
            header::X_FRAME_OPTIONS,
            HeaderValue::from_static("DENY"),
        ))
        .layer(SetResponseHeaderLayer::overriding(
            header::X_CONTENT_TYPE_OPTIONS,
            HeaderValue::from_static("nosniff"),
        ))
        .layer(SetResponseHeaderLayer::overriding(
            header::REFERRER_POLICY,
            HeaderValue::from_static("strict-origin-when-cross-origin"),
        ))
        // Middleware Layers
        .layer(CompressionLayer::new())
        .layer(session_layer)
        .with_state(state);

    let addr = SocketAddr::from(([0, 0, 0, 0], config.port));
    println!(
        "\n{}",
        green.apply_to(format!(
            "🔥 Bloogla running at http://localhost:{}",
            config.port
        ))
    );

    let listener = tokio::net::TcpListener::bind(addr).await?;
    axum::serve(
        listener,
        app.into_make_service_with_connect_info::<SocketAddr>(),
    )
    .await?;

    Ok(())
}

/// Runs CLI interactive prompts on initial deployment and populates SQLite
async fn run_cli_wizard(pool: &SqlitePool) -> Result<(), Box<dyn std::error::Error>> {
    let cyan = Style::new().cyan().bold();
    let green = Style::new().green().bold();
    let red = Style::new().red().bold();

    println!("{}", cyan.apply_to("First-time setup detected!\n"));

    let blog_name: String = Input::new()
        .with_prompt("Blog Title")
        .default("My Bloogla Site".into())
        .interact_text()?;

    let admin_email: String = Input::new().with_prompt("Admin Email").interact_text()?;

    // Prompt for password with baseline length validation
    let password = loop {
        let pass = Password::new()
            .with_prompt("Admin Password")
            .with_confirmation("Confirm Password", "Mismatch!")
            .interact()?;

        if pass.len() >= 8 {
            break pass;
        } else {
            println!(
                "{}",
                red.apply_to("Password must be at least 8 characters long.")
            );
        }
    };

    let salt = SaltString::generate(&mut thread_rng());
    let password_hash = Argon2::default()
        .hash_password(password.as_bytes(), &salt)
        .map_err(|e| format!("Password hashing failed: {e}"))?
        .to_string();

    // 1. Save admin credentials
    sqlx::query("INSERT INTO users (email, password_hash) VALUES (?, ?)")
        .bind(&admin_email)
        .bind(&password_hash)
        .execute(pool)
        .await?;

    // 2. Save dynamic initial settings
    let default_settings = [
        ("blog_name", blog_name.as_str()),
        (
            "blog_description",
            "A lightweight, high-performance blog built with Bloogla.",
        ),
        ("blog_keywords", "rust, bloogla, blog, webdev"),
        ("publisher_type", "Person"),
        ("publisher_name", &admin_email),
        ("articles_per_page", "10"),
    ];

    for (key, val) in default_settings {
        sqlx::query("INSERT INTO settings (key, value) VALUES (?, ?)")
            .bind(key)
            .bind(val)
            .execute(pool)
            .await?;
    }

    println!(
        "{}",
        green.apply_to("\n✔ Setup completed! Configuration stored in data/bloogla.db")
    );
    Ok(())
}
