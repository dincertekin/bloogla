//! `bloogla reset-password [EMAIL]`

use crate::app::security::{hash_password, log_event, missing_password_rules, PASSWORD_RULES};

use sqlx::SqlitePool;
use std::error::Error;

/// `bloogla reset-password [EMAIL]`: set a new password (for the first
/// account when no email is given) and sign everyone out.
pub async fn run(pool: &SqlitePool, email: Option<String>) -> Result<(), Box<dyn Error>> {
    let user: Option<(i64, String)> = match email {
        Some(email) => {
            sqlx::query_as("SELECT id, email FROM users WHERE email = ? COLLATE NOCASE")
                .bind(email)
                .fetch_optional(pool)
                .await?
        }
        None => {
            sqlx::query_as("SELECT id, email FROM users ORDER BY id ASC LIMIT 1")
                .fetch_optional(pool)
                .await?
        }
    };
    let (id, email) = user.ok_or("No matching account. Start the server to run setup.")?;

    println!("Setting a new password for {email}");
    println!("It needs:");
    for rule in PASSWORD_RULES {
        println!("  - {}", rule.text);
    }
    let password = loop {
        let password = dialoguer::Password::new()
            .with_prompt("New password")
            .with_confirmation("Confirm password", "Passwords do not match")
            .interact()?;
        let missing = missing_password_rules(&password);
        if missing.is_empty() {
            break password;
        }
        println!("That password still needs:");
        for rule in missing {
            println!("  - {}", rule.text);
        }
    };

    crate::db::users::set_password(pool, id, &hash_password(&password)?).await?;
    log_event("password_reset_from_command_line", &[("user", &email)]);
    // Sign out every existing session. The table only exists once the server
    // has run, so a missing table is fine.
    let _ = sqlx::query("DELETE FROM tower_sessions")
        .execute(pool)
        .await;

    println!("Password updated. All existing sessions were signed out.");
    Ok(())
}
