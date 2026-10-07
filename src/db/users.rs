//! People who can sign in: loading the person making a request (from their
//! session or an API token) and changing passwords.

use crate::app::models::{CurrentUser, Role};
use crate::db::{or_log, settings};
use crate::i18n::Lang;

use sqlx::SqlitePool;

type UserRow = (i64, String, String, String, String, i64);

async fn from_row(
    pool: &SqlitePool,
    (id, name, email, role, language, session_version): UserRow,
) -> CurrentUser {
    CurrentUser {
        id,
        name,
        email,
        role: Role::parse(&role).unwrap_or(Role::Author),
        lang: language_or_site(pool, &language).await,
        session_version,
    }
}

/// A person's chosen language, or the site's when they haven't picked one.
async fn language_or_site(pool: &SqlitePool, code: &str) -> Lang {
    match Lang::parse(code) {
        Some(lang) => lang,
        None => settings::load(pool).await.language,
    }
}

/// The person with this id (from the signed-in session).
pub async fn find(pool: &SqlitePool, id: i64) -> Option<CurrentUser> {
    let row: Option<UserRow> = or_log(
        sqlx::query_as(
            "SELECT id, name, email, role, language, session_version FROM users WHERE id = ?",
        )
        .bind(id)
        .fetch_optional(pool)
        .await,
        "load user",
    );
    Some(from_row(pool, row?).await)
}

/// The owner of an API token, given the token's hash. Records that it was used.
pub async fn find_by_token_hash(pool: &SqlitePool, token_hash: &str) -> Option<CurrentUser> {
    let row: Option<(i64, i64, String, String, String, String, i64)> = or_log(
        sqlx::query_as(
            "SELECT t.id, u.id, u.name, u.email, u.role, u.language, u.session_version
             FROM api_tokens t JOIN users u ON u.id = t.user_id WHERE t.token_hash = ?",
        )
        .bind(token_hash)
        .fetch_optional(pool)
        .await,
        "check api token",
    );
    let (token_id, id, name, email, role, language, version) = row?;

    let _ = sqlx::query("UPDATE api_tokens SET last_used_at = CURRENT_TIMESTAMP WHERE id = ?")
        .bind(token_id)
        .execute(pool)
        .await;
    Some(from_row(pool, (id, name, email, role, language, version)).await)
}

/// Set a new password and sign the account out on every device.
/// Returns the account's new session version.
pub async fn set_password(
    pool: &SqlitePool,
    id: i64,
    password_hash: &str,
) -> Result<i64, sqlx::Error> {
    sqlx::query_scalar(
        "UPDATE users SET password_hash = ?, session_version = session_version + 1
         WHERE id = ? RETURNING session_version",
    )
    .bind(password_hash)
    .bind(id)
    .fetch_one(pool)
    .await
}

// ---- Two-factor login ----

/// The authenticator key and the last time step used, when two-factor login is on.
pub async fn two_factor(pool: &SqlitePool, id: i64) -> Option<(Vec<u8>, u64)> {
    let row: Option<(Option<String>, i64)> = or_log(
        sqlx::query_as("SELECT totp_secret, totp_last_step FROM users WHERE id = ?")
            .bind(id)
            .fetch_optional(pool)
            .await,
        "load two-factor key",
    );
    let (stored, last_step) = row?;
    let secret = crate::app::security::from_hex(&crate::app::secrets::decrypt(&stored?)?)?;
    Some((secret, last_step.max(0) as u64))
}

/// Remember the time step a code was used in, so it can't be used again.
pub async fn set_two_factor_step(pool: &SqlitePool, id: i64, step: u64) {
    let _ = sqlx::query("UPDATE users SET totp_last_step = ? WHERE id = ? AND totp_last_step < ?")
        .bind(step as i64)
        .bind(id)
        .bind(step as i64)
        .execute(pool)
        .await;
}

/// Turn two-factor login on with `secret` (already confirmed with a code from
/// `step`), replacing any recovery codes with `recovery_hashes`.
pub async fn enable_two_factor(
    pool: &SqlitePool,
    id: i64,
    secret: &[u8],
    step: u64,
    recovery_hashes: &[String],
) -> Result<(), sqlx::Error> {
    let hex = crate::app::security::to_hex(secret);
    let mut tx = pool.begin().await?;
    sqlx::query("UPDATE users SET totp_secret = ?, totp_last_step = ? WHERE id = ?")
        .bind(crate::app::secrets::encrypt(&hex))
        .bind(step as i64)
        .bind(id)
        .execute(&mut *tx)
        .await?;
    replace_recovery_codes(&mut tx, id, recovery_hashes).await?;
    tx.commit().await
}

/// New recovery codes for someone who has two-factor login on.
pub async fn set_recovery_codes(
    pool: &SqlitePool,
    id: i64,
    hashes: &[String],
) -> Result<(), sqlx::Error> {
    let mut tx = pool.begin().await?;
    replace_recovery_codes(&mut tx, id, hashes).await?;
    tx.commit().await
}

async fn replace_recovery_codes(
    tx: &mut sqlx::Transaction<'_, sqlx::Sqlite>,
    id: i64,
    hashes: &[String],
) -> Result<(), sqlx::Error> {
    sqlx::query("DELETE FROM recovery_codes WHERE user_id = ?")
        .bind(id)
        .execute(&mut **tx)
        .await?;
    for hash in hashes {
        sqlx::query("INSERT INTO recovery_codes (user_id, code_hash) VALUES (?, ?)")
            .bind(id)
            .bind(hash)
            .execute(&mut **tx)
            .await?;
    }
    Ok(())
}

/// Turn two-factor login off and delete the recovery codes.
/// Returns false when the person doesn't exist.
pub async fn disable_two_factor(pool: &SqlitePool, id: i64) -> Result<bool, sqlx::Error> {
    let mut tx = pool.begin().await?;
    let changed =
        sqlx::query("UPDATE users SET totp_secret = NULL, totp_last_step = 0 WHERE id = ?")
            .bind(id)
            .execute(&mut *tx)
            .await?
            .rows_affected();
    replace_recovery_codes(&mut tx, id, &[]).await?;
    tx.commit().await?;
    Ok(changed > 0)
}

/// Use up a recovery code. True when it was valid and unused.
pub async fn use_recovery_code(pool: &SqlitePool, id: i64, code: &str) -> bool {
    let hash = crate::app::security::hash_recovery_code(code);
    let used = sqlx::query(
        "UPDATE recovery_codes SET used_at = CURRENT_TIMESTAMP
         WHERE user_id = ? AND code_hash = ? AND used_at IS NULL",
    )
    .bind(id)
    .bind(hash)
    .execute(pool)
    .await;
    matches!(used, Ok(r) if r.rows_affected() > 0)
}

/// Unused recovery codes left.
pub async fn recovery_codes_left(pool: &SqlitePool, id: i64) -> i64 {
    or_log(
        sqlx::query_scalar(
            "SELECT COUNT(*) FROM recovery_codes WHERE user_id = ? AND used_at IS NULL",
        )
        .bind(id)
        .fetch_one(pool)
        .await,
        "count recovery codes",
    )
}
