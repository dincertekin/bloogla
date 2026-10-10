//! People who can sign in: loading the person making a request (from their
//! session) and changing passwords.

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
        badges: Default::default(),
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

/// Save a profile only while the session that authorized it is current.
/// Changing the recovery address revokes older sessions and emailed links
/// together, so neither can restore access under the old address.
pub async fn update_profile(
    pool: &SqlitePool,
    me: &CurrentUser,
    name: &str,
    email: &str,
    language: &str,
) -> Result<Option<i64>, sqlx::Error> {
    let email_changed = email != me.email;
    let mut tx = pool.begin().await?;
    let version = sqlx::query_scalar(
        "UPDATE users SET name = ?, email = ?, language = ?, session_version = session_version + ?
         WHERE id = ? AND session_version = ? AND email = ? RETURNING session_version",
    )
    .bind(name)
    .bind(email)
    .bind(language)
    .bind(i64::from(email_changed))
    .bind(me.id)
    .bind(me.session_version)
    .bind(&me.email)
    .fetch_optional(&mut *tx)
    .await?;
    if version.is_some() && email_changed {
        sqlx::query("DELETE FROM password_resets WHERE user_id = ?")
            .bind(me.id)
            .execute(&mut *tx)
            .await?;
    }
    tx.commit().await?;
    Ok(version)
}

/// Set a new password and sign the account out on every device.
/// Returns the account's new session version.
pub async fn set_password(
    pool: &SqlitePool,
    id: i64,
    password_hash: &str,
) -> Result<i64, sqlx::Error> {
    change_password(pool, id, password_hash, None)
        .await?
        .ok_or(sqlx::Error::RowNotFound)
}

/// Change a password only if the session that checked the old password is
/// still current. A concurrent reset must win over this stale request.
pub async fn change_password(
    pool: &SqlitePool,
    id: i64,
    password_hash: &str,
    expected_version: Option<i64>,
) -> Result<Option<i64>, sqlx::Error> {
    let mut tx = pool.begin().await?;
    let version = sqlx::query_scalar(
        "UPDATE users SET password_hash = ?, session_version = session_version + 1
         WHERE id = ? AND (? IS NULL OR session_version = ?) RETURNING session_version",
    )
    .bind(password_hash)
    .bind(id)
    .bind(expected_version)
    .bind(expected_version)
    .fetch_optional(&mut *tx)
    .await?;
    if version.is_some() {
        // An older emailed link must never undo an ordinary or administrator
        // password change. Revocation and the change commit together.
        sqlx::query("DELETE FROM password_resets WHERE user_id = ?")
            .bind(id)
            .execute(&mut *tx)
            .await?;
    }
    tx.commit().await?;
    Ok(version)
}

/// Consume a valid reset link and change the password together. Only one
/// request can claim the link; a failed update leaves it available to retry.
pub async fn reset_password(
    pool: &SqlitePool,
    token_hash: &str,
    password_hash: &str,
) -> Result<Option<i64>, sqlx::Error> {
    let mut tx = pool.begin().await?;
    // The DELETE takes the write lock before checking the token, so concurrent
    // requests cannot both read it as unused.
    let user_id: Option<i64> = sqlx::query_scalar(
        "DELETE FROM password_resets WHERE token_hash = ? AND expires_at > datetime('now')
         RETURNING user_id",
    )
    .bind(token_hash)
    .fetch_optional(&mut *tx)
    .await?;
    let Some(id) = user_id else {
        tx.rollback().await?;
        return Ok(None);
    };
    sqlx::query(
        "UPDATE users SET password_hash = ?, session_version = session_version + 1 WHERE id = ?",
    )
    .bind(password_hash)
    .bind(id)
    .execute(&mut *tx)
    .await?;
    sqlx::query("DELETE FROM password_resets WHERE user_id = ?")
        .bind(id)
        .execute(&mut *tx)
        .await?;
    tx.commit().await?;
    Ok(Some(id))
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

/// Claim a code's time step. Only the request that advances it may sign in.
pub async fn set_two_factor_step(pool: &SqlitePool, id: i64, step: u64) -> bool {
    let saved =
        sqlx::query("UPDATE users SET totp_last_step = ? WHERE id = ? AND totp_last_step < ?")
            .bind(step as i64)
            .bind(id)
            .bind(step as i64)
            .execute(pool)
            .await;
    match saved {
        Ok(result) => result.rows_affected() == 1,
        Err(e) => {
            tracing::error!("Failed to record two-factor code use: {e}");
            false
        }
    }
}

/// Turn two-factor login on with `secret` (already confirmed with a code from
/// `step`), replacing any recovery codes with `recovery_hashes`.
pub async fn enable_two_factor(
    pool: &SqlitePool,
    id: i64,
    secret: &[u8],
    step: u64,
    recovery_hashes: &[String],
    expected_version: i64,
) -> Result<Option<i64>, sqlx::Error> {
    let hex = crate::app::security::to_hex(secret);
    let mut tx = pool.begin().await?;
    let version = sqlx::query_scalar(
        "UPDATE users SET totp_secret = ?, totp_last_step = ?, session_version = session_version + 1
         WHERE id = ? AND session_version = ? AND totp_secret IS NULL RETURNING session_version",
    )
        .bind(crate::app::secrets::encrypt(&hex))
        .bind(step as i64)
        .bind(id)
        .bind(expected_version)
        .fetch_optional(&mut *tx)
        .await?;
    if version.is_some() {
        replace_recovery_codes(&mut tx, id, recovery_hashes).await?;
    }
    tx.commit().await?;
    Ok(version)
}

/// New recovery codes for someone who has two-factor login on.
pub async fn set_recovery_codes(
    pool: &SqlitePool,
    id: i64,
    hashes: &[String],
    expected_version: i64,
) -> Result<Option<i64>, sqlx::Error> {
    let mut tx = pool.begin().await?;
    let version = sqlx::query_scalar(
        "UPDATE users SET session_version = session_version + 1
         WHERE id = ? AND session_version = ? AND totp_secret IS NOT NULL RETURNING session_version",
    )
    .bind(id)
    .bind(expected_version)
    .fetch_optional(&mut *tx)
    .await?;
    if version.is_some() {
        replace_recovery_codes(&mut tx, id, hashes).await?;
    }
    tx.commit().await?;
    Ok(version)
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
    disable_two_factor_if_current(pool, id, None).await
}

/// A password confirmed by a profile request must not authorize a change
/// after another request has revoked that session.
pub async fn disable_two_factor_if_current(
    pool: &SqlitePool,
    id: i64,
    expected_version: Option<i64>,
) -> Result<bool, sqlx::Error> {
    let mut tx = pool.begin().await?;
    let changed = sqlx::query(
        "UPDATE users SET totp_secret = NULL, totp_last_step = 0
         WHERE id = ? AND (? IS NULL OR session_version = ?)",
    )
    .bind(id)
    .bind(expected_version)
    .bind(expected_version)
    .execute(&mut *tx)
    .await?
    .rows_affected();
    if changed > 0 {
        replace_recovery_codes(&mut tx, id, &[]).await?;
    }
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
