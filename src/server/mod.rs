//! Starting the web server.
//!
//! - `routes`: every URL and the handler behind it
//! - `middleware`: security checks and other code that runs around handlers
//! - `assets`: the admin panel's CSS and JS, embedded in the binary
//! - `tls`: built-in HTTPS with Let's Encrypt

mod assets;
mod middleware;
pub mod routes;
mod tls;

use crate::app::config::Config;
use crate::app::console;
use crate::app::state::AppState;
use crate::db::settings;

use sqlx::SqlitePool;
use std::net::SocketAddr;
use std::sync::atomic::AtomicBool;
use std::sync::{Arc, RwLock};
use std::time::Duration;
use tower_sessions::session_store::ExpiredDeletion;
use tower_sessions::SessionManagerLayer;
use tower_sessions_sqlx_store::SqliteStore;

/// Log to stderr. In a terminal window a person is watching, only problems
/// are logged, as tidy lines (progress is shown by `app::console`).
/// Otherwise `BLOOGLA_LOG` sets the level filter (default `info`), and
/// `BLOOGLA_LOG_FORMAT=json` switches to one JSON object per line.
pub fn init_logging() {
    use tracing_subscriber::EnvFilter;

    if crate::app::console::for_a_person() {
        tracing_subscriber::fmt()
            .with_env_filter(EnvFilter::new("warn,sqlx=error,tower_sessions=error"))
            .with_writer(std::io::stderr)
            .event_format(crate::app::console::FriendlyLog)
            .init();
        return;
    }

    let filter = EnvFilter::try_from_env("BLOOGLA_LOG")
        .unwrap_or_else(|_| EnvFilter::new("info,sqlx=warn,tower_sessions=warn"));
    let builder = tracing_subscriber::fmt()
        .with_env_filter(filter)
        .with_writer(std::io::stderr)
        .with_ansi(std::io::IsTerminal::is_terminal(&std::io::stderr()))
        .with_target(false);

    if std::env::var("BLOOGLA_LOG_FORMAT").as_deref() == Ok("json") {
        builder.json().init();
    } else {
        builder.compact().init();
    }
}

/// `bloogla serve`: run the website until stopped (Ctrl+C or SIGTERM).
pub async fn run(config: Config, pool: SqlitePool) -> Result<(), Box<dyn std::error::Error>> {
    tracing::info!("Bloogla {}", env!("CARGO_PKG_VERSION"));

    if config.production && !config.base_url.starts_with("https://") {
        tracing::warn!(
            "BLOOGLA_PRODUCTION is on but BLOOGLA_BASE_URL is not https://. \
             Secure cookies are only sent over HTTPS, so admin login will not work over plain HTTP."
        );
    }

    for line in crate::services::themes::install_bundled()? {
        tracing::info!("{line}");
    }
    let setup_code = crate::handlers::admin::setup::prepare(&pool, &config.base_url).await?;

    let state = AppState {
        pool: pool.clone(),
        config: config.clone(),
        themes: Arc::new(RwLock::new(crate::services::themes::Themes::load())),
        setup_pending: Arc::new(AtomicBool::new(setup_code.is_some())),
        setup_code: setup_code.unwrap_or_default().into(),
        newer_release: Default::default(),
    };
    // Does nothing unless "Check for new versions every day" is on (Settings → Updates).
    crate::services::updates::spawn_daily_check(state.clone());

    // Sign-in sessions are stored in the database. Cookies are HTTP-only and
    // same-site; in production they're only sent over HTTPS.
    let session_store = SqliteStore::new(pool.clone());
    session_store.migrate().await?;
    let sessions = SessionManagerLayer::new(session_store.clone()).with_secure(config.production);

    spawn_maintenance(pool.clone(), session_store);
    // Post views are counted in memory and saved every few seconds.
    crate::handlers::site::analytics::spawn_view_saver(pool.clone());

    let app = routes::build(state.clone(), sessions)?;
    let addr = SocketAddr::new(config.host, config.port);
    let listener = listen(addr)?;
    // The start screen (or a log line on servers), once the site answers.
    // Settings are loaded first, so it speaks the site's language.
    settings::load(&pool).await;
    let folder = std::env::current_dir().unwrap_or_default();
    let ready = || {
        console::start_screen(console::StartScreen {
            site_url: &config.base_url,
            setup_url: state
                .setup_pending
                .load(std::sync::atomic::Ordering::Acquire)
                .then(|| format!("{}/setup?code={}", config.base_url, state.setup_code)),
            folder: &folder,
        })
    };
    match &config.tls {
        Some(tls_config) => tls::serve(app, listener, tls_config, shutdown_signal(), ready).await?,
        None => {
            tracing::info!("Listening on {addr} ({})", config.base_url);
            let listener = tokio::net::TcpListener::from_std(listener)?;
            ready();
            axum::serve(
                listener,
                app.into_make_service_with_connect_info::<SocketAddr>(),
            )
            .with_graceful_shutdown(shutdown_signal())
            .await?;
        }
    }

    crate::handlers::site::analytics::save_views(&pool).await;
    pool.close().await;
    console::stopped();
    // After installing an update: start the new version in this process's place.
    crate::services::updates::restart_if_updated()?;
    Ok(())
}

/// Start listening on `addr`, or explain plainly why that isn't possible
/// (the port is taken, or needs administrator rights).
pub fn listen(addr: SocketAddr) -> Result<std::net::TcpListener, String> {
    let listener =
        std::net::TcpListener::bind(addr).map_err(|e| console::port_problem(addr.port(), &e))?;
    // Tokio needs it non-blocking.
    listener.set_nonblocking(true).map_err(|e| e.to_string())?;
    Ok(listener)
}

/// Hourly housekeeping: the daily database backup and removing expired
/// sign-in sessions.
fn spawn_maintenance(pool: SqlitePool, session_store: SqliteStore) {
    let backups = crate::services::backup::automatic_backups_enabled();
    tokio::spawn(async move {
        // The first round runs a few seconds after starting, once the start
        // screen is shown, then every hour.
        let hour = Duration::from_secs(60 * 60);
        let first = tokio::time::Instant::now() + Duration::from_secs(5);
        let mut interval = tokio::time::interval_at(first, hour);
        loop {
            interval.tick().await;
            if backups {
                crate::services::backup::run_daily_backup(&pool).await;
            }
            if let Err(e) = session_store.delete_expired().await {
                tracing::error!("Could not remove expired sessions: {e}");
            }
        }
    });
}

/// Resolve on Ctrl+C or SIGTERM (sent by systemd and Docker on stop), or
/// when an installed update needs a restart.
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
        _ = crate::services::updates::restart_requested() => {},
    }
    console::stopping();
}
