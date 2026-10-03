//! `bloogla import-wordpress FILE`

use sqlx::SqlitePool;
use std::error::Error;

/// `bloogla import-wordpress FILE`: import a WordPress export (.xml).
pub async fn run(pool: &SqlitePool, file: &str) -> Result<(), Box<dyn Error>> {
    let xml = std::fs::read_to_string(file).map_err(|e| format!("Could not read {file}: {e}"))?;
    let report = crate::services::wordpress_import::import(pool, &xml).await?;

    println!(
        "Imported {} posts and {} pages, created {} tags.",
        report.posts, report.pages, report.tags_created
    );
    if !report.skipped_existing.is_empty() {
        println!(
            "  Skipped {} already present: {}",
            report.skipped_existing.len(),
            report.skipped_existing.join(", ")
        );
    }
    if report.skipped_other > 0 {
        println!(
            "  Skipped {} items that are not posts or pages (trash, menus, revisions...).",
            report.skipped_other
        );
    }
    println!(
        "  Images still point to your old site. Keep it online, or upload them in Admin → Media \
         and update the links."
    );
    Ok(())
}
