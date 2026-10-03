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
use crate::app::state::AppState;

use sqlx::SqlitePool;
use std::net::SocketAddr;
use std::sync::atomic::AtomicBool;
use std::sync::{Arc, RwLock};
use std::time::Duration;
use tower_sessions::session_store::ExpiredDeletion;
use tower_sessions::SessionManagerLayer;
use tower_sessions_sqlx_store::SqliteStore;

/// Log to stderr. `BLOOGLA_LOG` sets the level filter (default `info`), and
/// `BLOOGLA_LOG_FORMAT=json` switches to one JSON object per line.
pub fn init_logging() {
    use tracing_subscriber::EnvFilter;

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
    let setup_pending = crate::handlers::admin::setup::prepare(&pool, &config.base_url).await?;

    let state = AppState {
        pool: pool.clone(),
        config: config.clone(),
        tera: Arc::new(RwLock::new(crate::services::themes::load_templates()?)),
        setup_pending: Arc::new(AtomicBool::new(setup_pending)),
    };

    // Sign-in sessions are stored in the database. Cookies are HTTP-only and
    // same-site; in production they're only sent over HTTPS.
    let session_store = SqliteStore::new(pool.clone());
    session_store.migrate().await?;
    let sessions = SessionManagerLayer::new(session_store.clone()).with_secure(config.production);

    spawn_maintenance(pool.clone(), session_store);

    let app = routes::build(state, sessions)?;
    let addr = SocketAddr::new(config.host, config.port);
    match &config.tls {
        Some(tls_config) => tls::serve(app, addr, tls_config, shutdown_signal()).await?,
        None => {
            tracing::info!("Listening on {addr} ({})", config.base_url);
            let listener = tokio::net::TcpListener::bind(addr)
                .await
                .map_err(|e| format!("Could not listen on {addr}: {e}"))?;
            axum::serve(
                listener,
                app.into_make_service_with_connect_info::<SocketAddr>(),
            )
            .with_graceful_shutdown(shutdown_signal())
            .await?;
        }
    }

    pool.close().await;
    tracing::info!("Stopped");
    Ok(())
}

/// Hourly housekeeping: the daily database backup and removing expired
/// sign-in sessions.
fn spawn_maintenance(pool: SqlitePool, session_store: SqliteStore) {
    let backups = crate::services::backup::automatic_backups_enabled();
    tokio::spawn(async move {
        let mut interval = tokio::time::interval(Duration::from_secs(60 * 60));
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
    tracing::info!("Shutting down");
}
