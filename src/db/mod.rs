//! Everything that reads or writes the SQLite database.
//!
//! The tables themselves are created by the files in `migrations/`, which run
//! automatically on start.

pub mod fields;
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

    let pool = SqlitePoolOptions::new()
        .max_connections(10)
        .connect_with(options)
        .await?;

    sqlx::migrate!("./migrations").run(&pool).await?;

    Ok(pool)
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
