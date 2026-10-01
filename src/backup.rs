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

/// Back up once a day (starting now), keeping the newest [`KEEP_AUTOMATIC`] copies.
///
/// Disable with `BLOOGLA_AUTO_BACKUP=false`.
pub fn spawn_daily_backups(pool: SqlitePool) {
    if matches!(
        std::env::var("BLOOGLA_AUTO_BACKUP").as_deref(),
        Ok("false" | "0" | "no")
    ) {
        return;
    }

    tokio::spawn(async move {
        let mut interval = tokio::time::interval(Duration::from_secs(24 * 60 * 60));
        loop {
            interval.tick().await;
            match backup_to(&pool, &timestamped_path(AUTOMATIC_PREFIX)).await {
                Ok(()) => prune_automatic_backups(),
                Err(e) => eprintln!("Automatic backup failed: {e}"),
            }
        }
    });
}

fn prune_automatic_backups() {
    let Ok(entries) = std::fs::read_dir(BACKUP_DIR) else {
        return;
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

    // Timestamped names sort chronologically.
    automatic.sort();
    let excess = automatic.len().saturating_sub(KEEP_AUTOMATIC);
    for old in &automatic[..excess] {
        if let Err(e) = std::fs::remove_file(old) {
            eprintln!("Could not delete old backup {}: {e}", old.display());
        }
    }
}
