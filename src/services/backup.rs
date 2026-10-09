//! Backups: consistent copies of the SQLite database via `VACUUM INTO`
//! (daily, and with `bloogla backup`), and a downloadable .zip with the
//! database and uploaded images (Admin → Settings → Backup).

use crate::app::console::{self, Kind};
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
            console::activity_t(Kind::Done, "Daily backup saved.");
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

/// Make a .zip with a fresh copy of the database (`bloogla.db`) and every
/// uploaded file (`uploads/...`), and return where it was written.
///
/// The caller sends it and deletes it. Unpacking it into an empty folder
/// next to the `bloogla` program gives a working site again.
pub async fn download_archive(pool: &SqlitePool, uploads: &Path) -> Result<PathBuf, String> {
    let database = timestamped_path("download-");
    backup_to(pool, &database).await?;
    let archive = database.with_extension("zip");
    let uploads = uploads.to_path_buf();
    let (db, zip) = (database.clone(), archive.clone());
    let result = tokio::task::spawn_blocking(move || write_archive(&db, &uploads, &zip))
        .await
        .map_err(|e| e.to_string())
        .and_then(|r| r);
    let _ = std::fs::remove_file(&database);
    if result.is_err() {
        let _ = std::fs::remove_file(&archive);
    }
    result.map(|()| archive)
}

fn write_archive(database: &Path, uploads: &Path, archive: &Path) -> Result<(), String> {
    use std::io::Write;
    use zip::write::SimpleFileOptions;

    let error = |e: &dyn std::fmt::Display| format!("could not write the backup: {e}");
    let file = std::fs::File::create(archive).map_err(|e| error(&e))?;
    let mut zip = zip::ZipWriter::new(std::io::BufWriter::new(file));
    let mut add = |name: &str, path: &Path| -> Result<(), String> {
        let size = std::fs::metadata(path).map_err(|e| error(&e))?.len();
        let options = SimpleFileOptions::default()
            .compression_method(zip::CompressionMethod::Stored)
            .large_file(size >= u32::MAX as u64);
        zip.start_file(name, options).map_err(|e| error(&e))?;
        let mut source = std::fs::File::open(path).map_err(|e| error(&e))?;
        std::io::copy(&mut source, &mut zip).map_err(|e| error(&e))?;
        Ok(())
    };

    add("bloogla.db", database)?;
    if let Ok(entries) = std::fs::read_dir(uploads) {
        let mut files: Vec<PathBuf> = entries
            .flatten()
            .map(|e| e.path())
            .filter(|p| p.is_file())
            .collect();
        files.sort();
        for path in files {
            if let Some(name) = path.file_name().and_then(|n| n.to_str()) {
                add(&format!("uploads/{name}"), &path)?;
            }
        }
    }
    zip.finish()
        .map_err(|e| error(&e))?
        .flush()
        .map_err(|e| error(&e))
}
