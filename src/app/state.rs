use crate::app::config::Config;

use sqlx::SqlitePool;
use std::sync::atomic::AtomicBool;
use std::sync::{Arc, RwLock};
use tera::Tera;

/// Shared by every request handler (`State(state): State<AppState>`).
///
/// Cloning is cheap: every field is a handle to the same shared data.
#[derive(Clone)]
pub struct AppState {
    pub pool: SqlitePool,
    pub config: Config,
    /// Templates of every installed theme.
    pub tera: Arc<RwLock<Tera>>,
    /// True until the first admin account exists (browser setup is open).
    pub setup_pending: Arc<AtomicBool>,
}
