mod assets;
mod backup;
mod config;
mod db;
mod handlers;
mod import;
mod models;
mod seo;
mod setup;
mod tags;
mod templates;
mod themes;
mod utils;

use axum::extract::DefaultBodyLimit;
use axum::http::{header, HeaderName, HeaderValue};
use axum::{
    routing::{delete, get, post},
    Router,
};
use config::Config;
use console::Style;
use models::AppState;
use sqlx::SqlitePool;
use std::fs;
use std::net::SocketAddr;
use std::path::PathBuf;
use std::sync::atomic::AtomicBool;
use std::sync::{Arc, RwLock};
use tera::Tera;
use tower_governor::key_extractor::SmartIpKeyExtractor;
use tower_governor::{governor::GovernorConfigBuilder, GovernorLayer};
use tower_http::{
    compression::CompressionLayer, services::ServeDir, set_header::SetResponseHeaderLayer,
};
use tower_sessions::SessionManagerLayer;
use tower_sessions_sqlx_store::SqliteStore;

const HELP: &str = "\
Bloogla - a fast, single-binary blog engine

USAGE:
    bloogla [COMMAND]

COMMANDS:
    serve                     Start the web server (default)
    backup [FILE]             Save a copy of the database (default: data/backups/)
    reset-password [EMAIL]    Set a new admin password
    import-wordpress FILE     Import posts and pages from a WordPress export (.xml)
    help                      Show this message
    version                   Show the version

Configuration is read from BLOOGLA_* environment variables; see README.md.
";

#[tokio::main]
async fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let args: Vec<&str> = args.iter().map(String::as_str).collect();

    let result = match args.as_slice() {
        [] | ["serve"] => serve().await,
        ["backup"] => backup_command(None).await,
        ["backup", file] => backup_command(Some(file)).await,
        ["reset-password"] => reset_password_command(None).await,
        ["reset-password", email] => reset_password_command(Some(email)).await,
        ["import-wordpress", file] => import_command(file).await,
        ["help" | "--help" | "-h"] => {
            print!("{HELP}");
            Ok(())
        }
        ["version" | "--version" | "-V"] => {
            println!("bloogla {}", env!("CARGO_PKG_VERSION"));
            Ok(())
        }
        _ => {
            eprint!("Unknown command: {}\n\n{HELP}", args.join(" "));
            std::process::exit(2);
        }
    };

    if let Err(e) = result {
        eprintln!(
            "{}",
            Style::new().red().bold().apply_to(format!("Error: {e}"))
        );
        std::process::exit(1);
    }
}

type AppResult = Result<(), Box<dyn std::error::Error>>;

/// Load config and open (and migrate) the database.
async fn open_database() -> Result<(Config, SqlitePool), Box<dyn std::error::Error>> {
    fs::create_dir_all("data")?;
    let config = Config::from_env()?;
    let pool = db::init_db(&config.database_url).await?;
    Ok((config, pool))
}

async fn serve() -> AppResult {
    let bold = Style::new().bold();
    let green = Style::new().green();

    println!(
        "{}",
        bold.apply_to(format!("Bloogla {}", env!("CARGO_PKG_VERSION")))
    );

    let (config, pool) = open_database().await?;

    if config.production && !config.base_url.starts_with("https://") {
        eprintln!(
            "Warning: BLOOGLA_PRODUCTION is on but BLOOGLA_BASE_URL is not https://. \
             Secure cookies are only sent over HTTPS, so admin login will not work over plain HTTP."
        );
    }

    for line in assets::install_bundled_themes()? {
        println!("{line}");
    }

    let setup_pending = setup::prepare_first_run(&pool, &config.base_url).await?;

    let tera = Tera::new("themes/**/*.html").map_err(|e| format!("Theme parsing error: {e:?}"))?;

    let state = AppState {
        pool,
        config: config.clone(),
        tera: Arc::new(RwLock::new(tera)),
        setup_pending: Arc::new(AtomicBool::new(setup_pending)),
    };

    backup::spawn_daily_backups(state.pool.clone());

    // Session Layer
    let session_store = SqliteStore::new(state.pool.clone());
    session_store.migrate().await?;

    let session_layer = SessionManagerLayer::new(session_store).with_secure(config.production);

    // Rate-limit login attempts per client IP. Behind a local reverse proxy the
    // client IP comes from X-Forwarded-For; when exposed directly that header
    // could be forged, so the socket address is used instead.
    let login_post = if config.host.is_loopback() {
        let limit = GovernorConfigBuilder::default()
            .per_second(12)
            .burst_size(5)
            .key_extractor(SmartIpKeyExtractor)
            .finish()
            .ok_or("invalid rate limit config")?;
        post(handlers::auth::handle_login).layer(GovernorLayer {
            config: Arc::new(limit),
        })
    } else {
        let limit = GovernorConfigBuilder::default()
            .per_second(12)
            .burst_size(5)
            .finish()
            .ok_or("invalid rate limit config")?;
        post(handlers::auth::handle_login).layer(GovernorLayer {
            config: Arc::new(limit),
        })
    };

    // Uploads have random, never-reused names, so they can be cached forever
    let immutable_cache_layer = SetResponseHeaderLayer::overriding(
        header::CACHE_CONTROL,
        HeaderValue::from_static("public, max-age=31536000, immutable"),
    );

    // Axum Web Server Router
    let protected_admin_routes = Router::new()
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
        .route("/admin/settings/theme", post(handlers::admin::update_theme))
        .route(
            "/admin/posts",
            get(handlers::posts::list_posts).post(handlers::posts::create_post),
        )
        .route("/admin/posts/new", get(handlers::posts::new_post))
        .route(
            "/admin/posts/:id/edit",
            get(handlers::posts::edit_page).post(handlers::posts::update),
        )
        .route("/admin/posts/:id", delete(handlers::posts::delete))
        .route(
            "/admin/posts/:id/revisions/:rev",
            get(handlers::posts::revision),
        )
        .route(
            "/admin/pages",
            get(handlers::posts::list_pages).post(handlers::posts::create_page),
        )
        .route("/admin/pages/new", get(handlers::posts::new_page))
        .route(
            "/admin/pages/:id/edit",
            get(handlers::posts::edit_page).post(handlers::posts::update),
        )
        .route("/admin/pages/:id", delete(handlers::posts::delete))
        .route("/admin/preview", post(handlers::posts::preview))
        .route("/admin/tags", get(handlers::admin::tags_page))
        .route("/admin/tags/new", post(handlers::admin::create_tag))
        .route("/admin/tags/:id", delete(handlers::admin::delete_tag))
        .route(
            "/admin/media",
            get(handlers::media::media_page)
                .post(handlers::media::upload_media)
                .layer(DefaultBodyLimit::max(handlers::media::MAX_UPLOAD_BYTES)),
        )
        .route(
            "/admin/media/picker",
            get(handlers::media::picker)
                .post(handlers::media::picker_upload)
                .layer(DefaultBodyLimit::max(handlers::media::MAX_UPLOAD_BYTES)),
        )
        .route("/admin/media/:id", delete(handlers::media::delete_media))
        .route_layer(axum::middleware::from_fn(handlers::auth::auth_middleware));

    let app = Router::new()
        .merge(protected_admin_routes)
        .route("/admin/login", get(handlers::auth::login_page))
        .route("/admin/login", login_post)
        .route("/admin/logout", get(handlers::auth::handle_logout))
        .route("/setup", get(setup::setup_page).post(setup::handle_setup))
        // Admin assets are embedded in the binary
        .route("/static/*path", get(assets::serve_static))
        .nest(
            "/uploads",
            Router::new()
                .fallback_service(ServeDir::new(handlers::media::UPLOAD_DIR))
                .layer(immutable_cache_layer),
        )
        // Serve theme assets
        .nest_service("/theme-assets", ServeDir::new("themes"))
        // Public Endpoints
        .route("/", get(handlers::public::home_page))
        .route("/search", get(handlers::public::search_post))
        .route("/post/:slug", get(handlers::public::show_post))
        // Links from before posts were renamed
        .route("/article/:slug", get(handlers::public::legacy_article))
        .route("/tag/:slug", get(handlers::public::tag_page))
        .route("/health", get(handlers::public::health_check))
        .route("/rss.xml", get(handlers::public::rss_feed))
        .route("/sitemap.xml", get(handlers::public::sitemap_xml))
        .route("/robots.txt", get(handlers::public::robots_txt))
        .route("/favicon.ico", get(handlers::public::favicon))
        .route("/:slug", get(handlers::public::show_page))
        .fallback(handlers::public::fallback)
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
        .layer(SetResponseHeaderLayer::if_not_present(
            header::CONTENT_SECURITY_POLICY,
            HeaderValue::from_static(
                "frame-ancestors 'none'; base-uri 'self'; object-src 'none'; form-action 'self'",
            ),
        ))
        .layer(SetResponseHeaderLayer::overriding(
            HeaderName::from_static("permissions-policy"),
            HeaderValue::from_static(
                "camera=(), microphone=(), geolocation=(), interest-cohort=()",
            ),
        ))
        // Middleware Layers
        .layer(axum::middleware::from_fn(handlers::public::etag_middleware))
        .layer(axum::middleware::from_fn_with_state(
            state.clone(),
            setup::setup_guard,
        ))
        .layer(axum::middleware::from_fn_with_state(
            state.clone(),
            handlers::auth::csrf_guard,
        ))
        .layer(CompressionLayer::new())
        .layer(session_layer)
        .with_state(state.clone());

    // Only tell browsers to insist on HTTPS when the site is actually served over it.
    let app = if config.production {
        app.layer(SetResponseHeaderLayer::overriding(
            header::STRICT_TRANSPORT_SECURITY,
            HeaderValue::from_static("max-age=31536000"),
        ))
    } else {
        app
    };

    let addr = SocketAddr::new(config.host, config.port);
    println!(
        "{}",
        green.apply_to(format!("Listening on {addr} ({})", config.base_url))
    );

    let listener = tokio::net::TcpListener::bind(addr).await?;
    axum::serve(
        listener,
        app.into_make_service_with_connect_info::<SocketAddr>(),
    )
    .with_graceful_shutdown(shutdown_signal())
    .await?;

    state.pool.close().await;
    println!("Stopped.");
    Ok(())
}

/// Resolve on Ctrl+C or SIGTERM (sent by systemd and Docker on stop).
async fn shutdown_signal() {
    let ctrl_c = async {
        let _ = tokio::signal::ctrl_c().await;
    };

    #[cfg(unix)]
    let terminate = async {
        match tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate()) {
            Ok(mut signal) => {
                signal.recv().await;
            }
            Err(_) => std::future::pending::<()>().await,
        }
    };
    #[cfg(not(unix))]
    let terminate = std::future::pending::<()>();

    tokio::select! {
        _ = ctrl_c => {},
        _ = terminate => {},
    }
    println!("Shutting down...");
}

async fn backup_command(file: Option<&str>) -> AppResult {
    let (_, pool) = open_database().await?;
    let target = file
        .map(PathBuf::from)
        .unwrap_or_else(|| backup::timestamped_path(""));
    backup::backup_to(&pool, &target).await?;
    println!("Backup saved to {}", target.display());
    Ok(())
}

async fn reset_password_command(email: Option<&str>) -> AppResult {
    let (_, pool) = open_database().await?;

    let user: Option<(i64, String)> = match email {
        Some(email) => {
            sqlx::query_as("SELECT id, email FROM users WHERE email = ?")
                .bind(email)
                .fetch_optional(&pool)
                .await?
        }
        None => {
            sqlx::query_as("SELECT id, email FROM users ORDER BY id ASC LIMIT 1")
                .fetch_optional(&pool)
                .await?
        }
    };
    let (id, email) = user.ok_or("No matching admin account. Start the server to run setup.")?;

    println!("Setting a new password for {email}");
    let password = loop {
        let password = dialoguer::Password::new()
            .with_prompt("New password")
            .with_confirmation("Confirm password", "Passwords do not match")
            .interact()?;
        if password.chars().count() >= setup::MIN_PASSWORD_LEN {
            break password;
        }
        println!(
            "Password must be at least {} characters long.",
            setup::MIN_PASSWORD_LEN
        );
    };

    sqlx::query("UPDATE users SET password_hash = ? WHERE id = ?")
        .bind(setup::hash_password(&password)?)
        .bind(id)
        .execute(&pool)
        .await?;
    // Sign out every existing session.
    let _ = sqlx::query("DELETE FROM tower_sessions")
        .execute(&pool)
        .await;

    println!("Password updated. All existing sessions were signed out.");
    Ok(())
}

async fn import_command(file: &str) -> AppResult {
    let xml = fs::read_to_string(file).map_err(|e| format!("Could not read {file}: {e}"))?;
    let (_, pool) = open_database().await?;

    let report = import::import_wordpress(&pool, &xml).await?;

    println!(
        "Imported {} posts and {} pages, created {} tags.",
        report.posts, report.pages, report.tags_created
    );
    if !report.skipped_existing.is_empty() {
        println!(
            "  Skipped {} already present: {}",
            report.skipped_existing.len(),
            report.skipped_existing.join(", ")
        );
    }
    if report.skipped_other > 0 {
        println!(
            "  Skipped {} items that are not posts or pages (trash, menus, revisions...).",
            report.skipped_other
        );
    }
    println!(
        "  Images still point to your old site. Keep it online, or upload them in Admin → Media \
         and update the links."
    );
    Ok(())
}
