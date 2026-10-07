//! `bloogla disable-2fa EMAIL`: turn off two-factor login for someone who
//! lost their phone and their recovery codes (for example the only admin).

use crate::app::security::log_event;

use sqlx::SqlitePool;
use std::error::Error;

pub async fn run(pool: &SqlitePool, email: &str) -> Result<(), Box<dyn Error>> {
    let id: Option<i64> = sqlx::query_scalar("SELECT id FROM users WHERE email = ? COLLATE NOCASE")
        .bind(email)
        .fetch_optional(pool)
        .await?;
    let id = id.ok_or_else(|| format!("No account with the email {email}."))?;
    crate::db::users::disable_two_factor(pool, id).await?;
    log_event("two_factor_disabled_from_command_line", &[("user", email)]);
    println!("Two-factor login is off for {email}. They can sign in with just their password.");
    Ok(())
}
