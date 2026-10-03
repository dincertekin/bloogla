//! `bloogla backup [FILE]`

use sqlx::SqlitePool;
use std::error::Error;
use std::path::PathBuf;

/// `bloogla backup [FILE]`: save a consistent copy of the live database.
pub async fn run(pool: &SqlitePool, file: Option<String>) -> Result<(), Box<dyn Error>> {
    let target = file
        .map(PathBuf::from)
        .unwrap_or_else(|| crate::services::backup::timestamped_path(""));
    crate::services::backup::backup_to(pool, &target).await?;
    println!("Backup saved to {}", target.display());
    Ok(())
}
