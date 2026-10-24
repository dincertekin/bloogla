//! `bloogla reset-password [EMAIL]`

use crate::app::security::{hash_password, log_event, missing_password_rules, PASSWORD_RULES};
use crate::db::users;

use sqlx::SqlitePool;
use std::error::Error;

/// `bloogla reset-password [EMAIL]`: set a new password (for the first
/// account when no email is given) and sign everyone out.
pub async fn run(pool: &SqlitePool, email: Option<String>) -> Result<(), Box<dyn Error>> {
    let id = match email {
        Some(email) => users::id_by_email(pool, &email).await,
        None => users::owner_id(pool).await,
    };
    let id = id.ok_or("No matching account. Start the server to run setup.")?;
    let email = users::email_of(pool, id).await;

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

    users::set_password(pool, id, &hash_password(&password)?).await?;
    log_event("password_reset_from_command_line", &[("user", &email)]);
    users::forget_all_sessions(pool).await;

    println!("Password updated. All existing sessions were signed out.");
    Ok(())
}
