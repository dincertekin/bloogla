use crate::app::config::Config;
use crate::services::themes::Themes;

use sqlx::SqlitePool;
use std::sync::atomic::AtomicBool;
use std::sync::{Arc, RwLock};

/// Shared by every request handler (`State(state): State<AppState>`).
///
/// Cloning is cheap: every field is a handle to the same shared data.
#[derive(Clone)]
pub struct AppState {
    pub pool: SqlitePool,
    pub config: Config,
    /// Every installed theme with its templates.
    pub themes: Arc<RwLock<Themes>>,
    /// True until the first admin account exists (browser setup is open).
    pub setup_pending: Arc<AtomicBool>,
    /// The code browser setup needs, printed in the log as a setup link.
    /// Empty when setup isn't pending.
    pub setup_code: Arc<str>,
}
