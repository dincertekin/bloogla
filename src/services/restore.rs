//! Restoring a backup .zip (Settings → Backup → Restore), in three steps:
//!
//! 1. [`check`]: the uploaded .zip is unpacked into `data/restore/` and
//!    looked at. Nothing on the site changes yet.
//! 2. [`get_ready`]: after the admin confirms, a backup of the site as it is
//!    now is saved in `data/backups/`, and the unpacked backup is marked
//!    ready. Bloogla then restarts.
//! 3. [`finish_pending`]: on start, before the database is opened, the
//!    ready backup takes the place of `data/bloogla.db` and `uploads/`.
//!
//! Swapping files while the site runs would pull the database out from under
//! requests in progress; doing it during the restart is simple and safe.

use crate::app::console::{self, Kind};
use sqlx::sqlite::SqliteConnectOptions;
use sqlx::{ConnectOptions, Connection, SqliteConnection};
use std::io::Read;
use std::path::{Component, Path, PathBuf};

/// Largest backup .zip accepted, and the most its files may add up to.
pub const MAX_BACKUP_BYTES: u64 = 4 << 30; // 4 GB

/// Where an uploaded backup is unpacked, inside the site's folder.
const STAGING: &str = "data/restore";
/// Written into [`STAGING`] once the admin has confirmed the restore.
const READY: &str = "ready";

// What people see when a backup can't be used. Plain sentences, translated
// in the admin like any other message.
pub const NOT_A_BACKUP: &str =
    "This file isn't a Bloogla backup. Choose a .zip made with Download backup.";
pub const DAMAGED: &str = "This backup is damaged and can't be read. Try another copy.";
pub const TOO_LARGE: &str =
    "This backup is larger than 4 GB, which is more than Bloogla can restore.";
pub const NEWER: &str =
    "This backup was made by a newer version of Bloogla. Update Bloogla first (Updates tab), then restore it.";
pub const NO_ADMIN: &str =
    "This backup has no admin account, so nobody could sign in after restoring it.";
pub const NO_SPACE: &str =
    "Couldn't unpack the backup on the server. Check that there's enough free disk space, then try again.";

/// What a checked backup contains, to show before restoring it.
pub struct Summary {
    pub posts: i64,
    pub pages: i64,
    pub people: i64,
    pub images: i64,
    /// Title and publish date (as stored) of the newest post, if any.
    pub newest_post: Option<(String, String)>,
}

/// Step 1: unpack the backup at `upload` into `data/restore/` and look at
/// it. `Err` is one of the messages above.
pub async fn check(folder: &Path, upload: &Path) -> Result<Summary, &'static str> {
    let staging = folder.join(STAGING);
    let _ = std::fs::remove_dir_all(&staging);
    let (from, to) = (upload.to_path_buf(), staging.clone());
    let unpacked = tokio::task::spawn_blocking(move || unpack(&from, &to))
        .await
        .unwrap_or(Err(DAMAGED));
    let summary = match unpacked {
        Ok(()) => inspect(&staging.join("bloogla.db")).await,
        Err(message) => Err(message),
    };
    if summary.is_err() {
        let _ = std::fs::remove_dir_all(&staging);
    }
    summary
}

/// Whether a checked backup is waiting for the admin to confirm it.
pub fn is_checked(folder: &Path) -> bool {
    folder.join(STAGING).join("bloogla.db").is_file()
}

/// Forget a checked backup (the admin chose Cancel).
pub fn cancel(folder: &Path) {
    let _ = std::fs::remove_dir_all(folder.join(STAGING));
}

/// Step 2: save the site as it is now (`data/backups/before-restore-…zip`,
/// returned) and mark the checked backup ready for the next start.
pub async fn get_ready(pool: &sqlx::SqlitePool, folder: &Path) -> Result<PathBuf, String> {
    if !is_checked(folder) {
        return Err("no checked backup".into());
    }
    let stamp = chrono::Utc::now().format("%Y%m%d-%H%M%S");
    let safety = folder
        .join(super::backup::BACKUP_DIR)
        .join(format!("before-restore-{stamp}.zip"));
    if let Some(parent) = safety.parent() {
        std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
    }
    super::backup::write_backup(pool, &folder.join("uploads"), &safety).await?;
    std::fs::write(
        folder.join(STAGING).join(READY),
        safety.display().to_string(),
    )
    .map_err(|e| e.to_string())?;
    Ok(safety)
}

/// Step 3, on start before the database is opened: put a ready backup in
/// place. A backup that was checked but never confirmed is thrown away.
pub fn finish_pending(folder: &Path) -> Result<(), String> {
    let staging = folder.join(STAGING);
    if !staging.exists() {
        return Ok(());
    }
    let Ok(safety) = std::fs::read_to_string(staging.join(READY)) else {
        cancel(folder);
        return Ok(());
    };
    put_in_place(folder, &staging).map_err(|e| {
        format!(
            "Couldn't finish restoring the backup ({e}). Your site as it was before is saved in {safety}."
        )
    })?;
    let _ = std::fs::remove_dir_all(&staging);
    console::activity_t(Kind::Done, "Backup restored.");
    Ok(())
}

fn put_in_place(folder: &Path, staging: &Path) -> std::io::Result<()> {
    let data = folder.join("data");
    // The old database's journal must go with it: SQLite would otherwise
    // replay it into the restored database.
    for leftover in ["bloogla.db-wal", "bloogla.db-shm"] {
        let path = data.join(leftover);
        if path.exists() {
            std::fs::remove_file(path)?;
        }
    }
    let database = data.join("bloogla.db");
    if database.exists() {
        // Windows can't rename onto an existing file.
        std::fs::remove_file(&database)?;
    }
    std::fs::rename(staging.join("bloogla.db"), &database)?;

    // The images: the old ones are in the safety backup.
    let uploads = folder.join("uploads");
    if uploads.exists() {
        std::fs::remove_dir_all(&uploads)?;
    }
    let restored = staging.join("uploads");
    if restored.exists() {
        move_folder(&restored, &uploads)?;
    } else {
        std::fs::create_dir_all(&uploads)?;
    }
    Ok(())
}

/// Move a folder of files, copying when renaming isn't possible (a different
/// disk, as Docker setups sometimes have).
fn move_folder(from: &Path, to: &Path) -> std::io::Result<()> {
    if std::fs::rename(from, to).is_ok() {
        return Ok(());
    }
    std::fs::create_dir_all(to)?;
    for entry in std::fs::read_dir(from)? {
        let path = entry?.path();
        if let Some(name) = path.file_name() {
            std::fs::copy(&path, to.join(name))?;
        }
    }
    std::fs::remove_dir_all(from)
}

/// Where a file in the .zip goes inside `staging`, or `None` to leave it
/// out. Only the database and the images are taken; anything else (a
/// `__MACOSX` folder from re-zipping on a Mac, or files someone added) is
/// ignored, and names that try to leave the folder never match.
fn destination(name: &Path, staging: &Path) -> Option<PathBuf> {
    let parts: Vec<&str> = name
        .components()
        .map(|part| match part {
            Component::Normal(part) => part.to_str(),
            _ => None,
        })
        .collect::<Option<_>>()?;
    match parts.as_slice() {
        // Backups from Bloogla 1.0.0 had the database at the top.
        ["data", "bloogla.db"] | ["bloogla.db"] => Some(staging.join("bloogla.db")),
        ["uploads", file] if !file.starts_with('.') => Some(staging.join("uploads").join(file)),
        _ => None,
    }
}

fn unpack(upload: &Path, staging: &Path) -> Result<(), &'static str> {
    let file = std::fs::File::open(upload).map_err(|_| NO_SPACE)?;
    let mut archive =
        zip::ZipArchive::new(std::io::BufReader::new(file)).map_err(|_| NOT_A_BACKUP)?;
    std::fs::create_dir_all(staging.join("uploads")).map_err(|_| NO_SPACE)?;
    let mut total = 0u64;
    for index in 0..archive.len() {
        let mut entry = archive.by_index(index).map_err(|_| DAMAGED)?;
        if entry.is_dir() {
            continue;
        }
        let Some(target) = entry
            .enclosed_name()
            .and_then(|name| destination(&name, staging))
        else {
            continue;
        };
        let mut out = std::fs::File::create(&target).map_err(|_| NO_SPACE)?;
        // Count what's really unpacked rather than trusting the sizes the
        // .zip claims, so a small file can't unpack into a huge one.
        let room = MAX_BACKUP_BYTES - total;
        let copied = std::io::copy(&mut (&mut entry).take(room + 1), &mut out).map_err(|e| {
            if e.kind() == std::io::ErrorKind::StorageFull {
                NO_SPACE
            } else {
                DAMAGED
            }
        })?;
        if copied > room {
            return Err(TOO_LARGE);
        }
        total += copied;
    }
    if !staging.join("bloogla.db").is_file() {
        return Err(NOT_A_BACKUP);
    }
    Ok(())
}

/// Check the unpacked database and count what's in it.
async fn inspect(database: &Path) -> Result<Summary, &'static str> {
    let options = SqliteConnectOptions::new()
        .filename(database)
        .disable_statement_logging();
    let mut db = SqliteConnection::connect_with(&options)
        .await
        .map_err(|_| NOT_A_BACKUP)?;
    let summary = look_inside(&mut db).await;
    // Closing writes everything back into the one file.
    let _ = db.close().await;
    summary
}

async fn look_inside(db: &mut SqliteConnection) -> Result<Summary, &'static str> {
    // Something that isn't a database fails here.
    let health: String = sqlx::query_scalar("PRAGMA quick_check")
        .fetch_one(&mut *db)
        .await
        .map_err(|_| NOT_A_BACKUP)?;
    if health != "ok" {
        return Err(DAMAGED);
    }
    let applied: Vec<i64> = sqlx::query_scalar("SELECT version FROM _sqlx_migrations")
        .fetch_all(&mut *db)
        .await
        .map_err(|_| NOT_A_BACKUP)?;
    let known: Vec<i64> = crate::db::MIGRATOR.iter().map(|m| m.version).collect();
    if applied.is_empty() {
        return Err(NOT_A_BACKUP);
    }
    if applied.iter().any(|version| !known.contains(version)) {
        return Err(NEWER);
    }

    let count = |sql: &'static str| sqlx::query_scalar::<_, i64>(sql);
    let admins = count("SELECT COUNT(*) FROM users WHERE role = 'admin'")
        .fetch_one(&mut *db)
        .await
        .map_err(|_| DAMAGED)?;
    if admins == 0 {
        return Err(NO_ADMIN);
    }
    let posts = count("SELECT COUNT(*) FROM posts WHERE is_page = 0")
        .fetch_one(&mut *db)
        .await
        .map_err(|_| DAMAGED)?;
    let pages = count("SELECT COUNT(*) FROM posts WHERE is_page = 1")
        .fetch_one(&mut *db)
        .await
        .map_err(|_| DAMAGED)?;
    let people = count("SELECT COUNT(*) FROM users")
        .fetch_one(&mut *db)
        .await
        .map_err(|_| DAMAGED)?;
    let images = count("SELECT COUNT(*) FROM media")
        .fetch_one(&mut *db)
        .await
        .map_err(|_| DAMAGED)?;
    let newest_post = sqlx::query_as::<_, (String, String)>(
        "SELECT title, published_at FROM posts WHERE is_page = 0
         ORDER BY published_at DESC LIMIT 1",
    )
    .fetch_optional(&mut *db)
    .await
    .map_err(|_| DAMAGED)?;
    Ok(Summary {
        posts,
        pages,
        people,
        images,
        newest_post,
    })
}
