//! Everything that reads or writes the SQLite database.
//!
//! The tables themselves are created by the files in `migrations/`, which run
//! automatically on start.

pub mod posts;
pub mod settings;
pub mod tags;
pub mod users;

use sqlx::sqlite::{SqliteConnectOptions, SqliteJournalMode, SqlitePoolOptions, SqliteSynchronous};
use sqlx::SqlitePool;
use std::str::FromStr;
use std::time::Duration;

/// Open the database (creating it if needed) and apply any new migrations.
pub async fn connect(database_url: &str) -> Result<SqlitePool, Box<dyn std::error::Error>> {
    let options = SqliteConnectOptions::from_str(database_url)?
        // Readers don't block the writer, and the writer doesn't block readers.
        .journal_mode(SqliteJournalMode::Wal)
        .synchronous(SqliteSynchronous::Normal)
        .busy_timeout(Duration::from_secs(5))
        .foreign_keys(true);

    let filename = options.get_filename().to_path_buf();
    let on_disk =
        filename != std::path::Path::new(":memory:") && !database_url.contains("mode=memory");
    if on_disk {
        if !filename.exists() && database_url.contains("mode=rwc") {
            // SQLite would otherwise create the database using the umask.
            match crate::app::private_files::create_file(&filename) {
                Ok(_) => {}
                Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => {}
                Err(e) => return Err(e.into()),
            }
        }
        crate::app::private_files::protect_database(&filename)?;
    }

    let pool = SqlitePoolOptions::new()
        .max_connections(10)
        .connect_with(options)
        .await?;

    if on_disk {
        // WAL and shared-memory files inherit the database's permissions.
        crate::app::private_files::protect_database(&filename)?;
    }
    let migrator = sqlx::migrate!("./migrations");
    refuse_pre_release_database(&pool, &migrator).await?;
    migrator.run(&pool).await?;

    Ok(pool)
}

/// Databases made by development versions of Bloogla (before the schema was
/// merged into one file) can't be upgraded. Say so plainly instead of
/// sqlx's "previously applied but is missing" error.
async fn refuse_pre_release_database(
    pool: &SqlitePool,
    migrator: &sqlx::migrate::Migrator,
) -> Result<(), String> {
    let applied: Vec<i64> = sqlx::query_scalar("SELECT version FROM _sqlx_migrations")
        .fetch_all(pool)
        .await
        .unwrap_or_default(); // No table yet: a new database.
    let known: Vec<i64> = migrator.iter().map(|m| m.version).collect();
    if applied.iter().any(|version| !known.contains(version)) {
        return Err(
            "This database was made by a development version of Bloogla and can't be \
                    upgraded. Move data/bloogla.db somewhere safe and start again to create a \
                    fresh one."
                .into(),
        );
    }
    Ok(())
}

/// Unwrap a database result, logging the error and falling back to the default value.
///
/// Used where a failed query should show an empty list rather than an error page.
pub fn or_log<T: Default>(result: Result<T, sqlx::Error>, context: &str) -> T {
    result.unwrap_or_else(|e| {
        tracing::error!("Database error ({context}): {e}");
        T::default()
    })
}
