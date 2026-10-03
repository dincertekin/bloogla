//! Consistent hot backups of the SQLite database via `VACUUM INTO`.

use sqlx::SqlitePool;
use std::path::{Path, PathBuf};
use std::time::Duration;

pub const BACKUP_DIR: &str = "data/backups";
/// Automatic backups kept on disk; older ones are deleted.
const KEEP_AUTOMATIC: usize = 7;
const AUTOMATIC_PREFIX: &str = "auto-";

/// Write a consistent copy of the live database to `target`.
pub async fn backup_to(pool: &SqlitePool, target: &Path) -> Result<(), String> {
    if target.exists() {
        return Err(format!("{} already exists", target.display()));
    }
    if let Some(parent) = target.parent() {
        std::fs::create_dir_all(parent)
            .map_err(|e| format!("could not create {}: {e}", parent.display()))?;
    }

    sqlx::query("VACUUM INTO ?")
        .bind(target.to_string_lossy().as_ref())
        .execute(pool)
        .await
        .map(|_| ())
        .map_err(|e| format!("backup failed: {e}"))
}

/// Default path for a backup taken now, e.g. `data/backups/bloogla-20261002-101500.db`.
pub fn timestamped_path(prefix: &str) -> PathBuf {
    let stamp = chrono::Utc::now().format("%Y%m%d-%H%M%S");
    PathBuf::from(BACKUP_DIR).join(format!("{prefix}bloogla-{stamp}.db"))
}

/// Whether daily automatic backups are on (`BLOOGLA_AUTO_BACKUP`, default yes).
pub fn automatic_backups_enabled() -> bool {
    !matches!(
        std::env::var("BLOOGLA_AUTO_BACKUP").as_deref(),
        Ok("false" | "0" | "no")
    )
}

/// Take an automatic backup if the newest one is a day old, keeping the
/// newest [`KEEP_AUTOMATIC`]. Called hourly, so frequent restarts don't
/// replace older backups with near copies.
pub async fn run_daily_backup(pool: &SqlitePool) {
    let due = newest_automatic_backup_age().is_none_or(|age| age >= DAY);
    if !due {
        return;
    }
    match backup_to(pool, &timestamped_path(AUTOMATIC_PREFIX)).await {
        Ok(()) => {
            tracing::info!("Automatic backup saved");
            prune_automatic_backups();
        }
        Err(e) => tracing::error!("Automatic backup failed: {e}"),
    }
}

const DAY: Duration = Duration::from_secs(24 * 60 * 60);

/// Paths of automatic backups, oldest first (timestamped names sort chronologically).
fn automatic_backups() -> Vec<PathBuf> {
    let Ok(entries) = std::fs::read_dir(BACKUP_DIR) else {
        return Vec::new();
    };
    let mut automatic: Vec<PathBuf> = entries
        .flatten()
        .map(|e| e.path())
        .filter(|p| {
            p.file_name()
                .and_then(|n| n.to_str())
                .is_some_and(|n| n.starts_with(AUTOMATIC_PREFIX) && n.ends_with(".db"))
        })
        .collect();
    automatic.sort();
    automatic
}

fn newest_automatic_backup_age() -> Option<Duration> {
    let newest = automatic_backups().pop()?;
    std::fs::metadata(newest)
        .ok()?
        .modified()
        .ok()?
        .elapsed()
        .ok()
}

fn prune_automatic_backups() {
    let automatic = automatic_backups();
    let excess = automatic.len().saturating_sub(KEEP_AUTOMATIC);
    for old in &automatic[..excess] {
        if let Err(e) = std::fs::remove_file(old) {
            tracing::error!("Could not delete old backup {}: {e}", old.display());
        }
    }
}
