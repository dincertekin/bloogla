mod config;
mod db;
mod handlers;
mod models;
mod tags;
mod templates;
mod utils;

use axum::http::{header, HeaderValue};
use axum::{
    routing::{delete, get, post},
    Router,
};
use config::Config;
use console::Style;
use dialoguer::{Input, Password};
use models::AppState;
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

    println!(
        "{}",
        cyan.apply_to("\n🚀 Welcome to Bloogla - Ultra Fast Rust CMS\n")
    );

    let config_path = Path::new("bloogla.json");
    let db_path = "sqlite://bloogla.db?mode=rwc";

    // CLI First Setup Wizard
    let config = if !config_path.exists() {
        println!("{}", cyan.apply_to("First-time setup detected!\n"));

        let blog_name: String = Input::new()
            .with_prompt("Blog Title")
            .default("My Bloogla Site".into())
            .interact_text()?;
        let admin_email: String = Input::new().with_prompt("Admin Email").interact_text()?;
        let password = Password::new()
            .with_prompt("Admin Password")
            .with_confirmation("Confirm Password", "Mismatch!")
            .interact()?;
        let port: u16 = Input::new()
            .with_prompt("Server Port")
            .default(8080)
            .interact_text()?;
        let production: bool = Input::new()
            .with_prompt("Running behind HTTPS in production? (true/false)")
            .default(false)
            .interact_text()?;
        let base_url: String = Input::new()
            .with_prompt("Site URL (e.g. https://blog.example.com)")
            .default(format!("http://localhost:{}", port))
            .interact_text()?;

        use argon2::password_hash::{PasswordHasher, SaltString};
        use argon2::Argon2;
        use rand::thread_rng;

        let salt = SaltString::generate(&mut thread_rng());
        let password_hash = Argon2::default()
            .hash_password(password.as_bytes(), &salt)
            .map_err(|e| format!("Password hashing failed: {e}"))?
            .to_string();

        let cfg = Config {
            blog_name,
            admin_email,
            admin_password_hash: password_hash,
            port,
            production,
            base_url,
        };
        fs::write(config_path, serde_json::to_string_pretty(&cfg)?)?;
        println!("{}", green.apply_to("\n✔ Configuration saved!"));
        cfg
    } else {
        let content = fs::read_to_string(config_path)?;
        serde_json::from_str(&content)?
    };

    // Database and Migration Start
    let pool = db::init_db(db_path).await?;
    println!("{}", green.apply_to("✔ Database migrations applied!"));

    let state = AppState {
        pool,
        config: config.clone(),
    };

    // Session Layer
    let session_store = MemoryStore::default();
    let session_layer = SessionManagerLayer::new(session_store).with_secure(config.production);

    // Rate Limiter
    let governor_conf = Arc::new(
        GovernorConfigBuilder::default()
            .per_second(12)
            .burst_size(5)
            .key_extractor(SmartIpKeyExtractor)
            .finish()
            .unwrap(),
    );

    // 1 year caching for static files
    let static_cache_layer = SetResponseHeaderLayer::overriding(
        header::CACHE_CONTROL,
        HeaderValue::from_static("public, max-age=31536000, immutable"),
    );

    // Axum Web Server Routes
    let app = Router::new()
        .nest(
            "/static",
            Router::new()
                .fallback_service(ServeDir::new("static"))
                .layer(static_cache_layer),
        )
        .route("/", get(handlers::public::home_page))
        .route("/post/:slug", get(handlers::public::show_post))
        .route("/tag/:slug", get(handlers::public::tag_page))
        .route("/health", get(handlers::public::health_check))
        .route("/rss.xml", get(handlers::public::rss_feed))
        .route("/sitemap.xml", get(handlers::public::sitemap_xml))
        // Admin Pages
        .route("/admin/login", get(handlers::auth::login_page))
        .route(
            "/admin/login",
            post(handlers::auth::handle_login).layer(GovernorLayer {
                config: governor_conf,
            }),
        )
        .route("/admin/logout", get(handlers::auth::handle_logout))
        // Post Management (CRUD) Routes
        .route("/admin", get(handlers::admin::admin_dashboard))
        .route("/admin/posts", get(handlers::admin::posts_page))
        .route("/admin/posts/new", post(handlers::admin::create_post))
        .route(
            "/admin/posts/:id/edit",
            get(handlers::admin::edit_post_page).post(handlers::admin::handle_edit_post),
        )
        .route("/admin/posts/:id", delete(handlers::admin::delete_post))
        // Tag Management (CRUD) Routes
        .route("/admin/tags", get(handlers::admin::tags_page))
        .route("/admin/tags/new", post(handlers::admin::create_tag))
        .route("/admin/tags/:id", delete(handlers::admin::delete_tag))
        // Compression
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
