//! Backups: consistent copies of the SQLite database via `VACUUM INTO`
//! (daily, and with `bloogla backup`), and a downloadable .zip with the
//! database and uploaded images (Admin → Settings → Backup).

use crate::app::console::{self, Kind};
use crate::app::private_files;
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
    let parent = target
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        .unwrap_or(Path::new("."));
    private_files::create_parents(parent).map_err(|e| e.to_string())?;
    // VACUUM INTO requires a missing file and chooses its own permissions.
    // Keep that intermediate file inside a private directory, then publish
    // the completed, protected backup without overwriting another file.
    let staging = parent.join(format!(
        ".bloogla-backup-{}",
        crate::app::security::random_hex(16)
    ));
    private_files::create_directory(&staging).map_err(|e| e.to_string())?;
    let _cleanup = BackupStaging(staging.clone());
    let database = staging.join("backup.db");
    sqlx::query("VACUUM INTO ?")
        .bind(database.to_string_lossy().as_ref())
        .execute(pool)
        .await
        .map_err(|e| format!("backup failed: {e}"))?;
    private_files::protect_file(&database).map_err(|e| e.to_string())?;
    let target = target.to_path_buf();
    tokio::task::spawn_blocking(move || publish_backup(&database, &target))
        .await
        .map_err(|e| format!("backup failed: {e}"))?
}

fn publish_backup(database: &Path, target: &Path) -> Result<(), String> {
    if std::fs::hard_link(database, target).is_ok() {
        return Ok(());
    }
    // Removable drives may not support hard links. A new private file is
    // safe to copy into, and create_new still refuses to replace any file.
    let error = |e| format!("backup failed: {e}");
    let mut source = std::fs::File::open(database).map_err(error)?;
    let mut destination = private_files::create_file(target).map_err(error)?;
    if let Err(e) = std::io::copy(&mut source, &mut destination) {
        drop(destination);
        let _ = std::fs::remove_file(target);
        return Err(error(e));
    }
    Ok(())
}

struct BackupStaging(PathBuf);

impl Drop for BackupStaging {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

/// Protect backups saved by older releases, too. Only touch Bloogla's own
/// directory; an explicitly chosen CLI destination may be shared.
pub fn protect_backup_directory() -> std::io::Result<()> {
    let directory = Path::new(BACKUP_DIR);
    private_files::create_directory(directory)?;
    for entry in std::fs::read_dir(directory)? {
        let path = entry?.path();
        if path.is_file()
            && path
                .extension()
                .is_some_and(|ext| ext == "db" || ext == "zip")
        {
            private_files::protect_file(&path)?;
        }
    }
    Ok(())
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
    if let Err(e) = protect_backup_directory() {
        tracing::error!("Could not protect backup directory: {e}");
        return;
    }
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

/// Make a .zip with a fresh copy of the database (`data/bloogla.db`) and every
/// uploaded file (`uploads/...`), and return where it was written.
///
/// The caller sends it and deletes it. Unpacking it into an empty folder
/// next to the `bloogla` program gives a working site again.
pub async fn download_archive(pool: &SqlitePool, uploads: &Path) -> Result<PathBuf, String> {
    protect_backup_directory().map_err(|e| e.to_string())?;
    let database = PathBuf::from(BACKUP_DIR).join(format!(
        "download-{}.db",
        crate::app::security::random_hex(16)
    ));
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
    let file = private_files::create_file(archive).map_err(|e| error(&e))?;
    let mut zip = zip::ZipWriter::new(std::io::BufWriter::new(file));
    let mut add = |name: &str, path: &Path| -> Result<(), String> {
        let size = std::fs::metadata(path).map_err(|e| error(&e))?.len();
        let options = SimpleFileOptions::default()
            .compression_method(zip::CompressionMethod::Stored)
            .large_file(size >= u32::MAX as u64)
            .unix_permissions(if name == "data/bloogla.db" {
                0o600
            } else {
                0o644
            });
        zip.start_file(name, options).map_err(|e| error(&e))?;
        let mut source = std::fs::File::open(path).map_err(|e| error(&e))?;
        std::io::copy(&mut source, &mut zip).map_err(|e| error(&e))?;
        Ok(())
    };

    // Match the path opened on startup so unpacking the backup restores the site.
    add("data/bloogla.db", database)?;
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
