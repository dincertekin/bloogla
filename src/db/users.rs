//! People who can sign in: loading the person making a request (from their
//! session) and changing passwords.

use crate::app::models::{ColorScheme, CurrentUser, Role};
use crate::db::{or_log, settings};
use crate::i18n::Lang;

use sqlx::SqlitePool;

type UserRow = (i64, String, String, String, String, i64, String);

async fn from_row(
    pool: &SqlitePool,
    (id, name, email, role, language, session_version, color_scheme): UserRow,
) -> CurrentUser {
    CurrentUser {
        id,
        name,
        email,
        role: Role::parse(&role).unwrap_or(Role::Author),
        lang: language_or_site(pool, &language).await,
        session_version,
        color_scheme: ColorScheme::parse(&color_scheme).unwrap_or_default(),
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
            "SELECT id, name, email, role, language, session_version, color_scheme
             FROM users WHERE id = ?",
        )
        .bind(id)
        .fetch_optional(pool)
        .await,
        "load user",
    );
    Some(from_row(pool, row?).await)
}

/// Save someone's light or dark choice for the admin panel.
pub async fn set_color_scheme(
    pool: &SqlitePool,
    id: i64,
    scheme: ColorScheme,
) -> Result<(), sqlx::Error> {
    sqlx::query("UPDATE users SET color_scheme = ? WHERE id = ?")
        .bind(scheme.as_str())
        .bind(id)
        .execute(pool)
        .await
        .map(|_| ())
}

/// Save someone's name, email and language. A new email signs the account
/// out on every device and cancels emailed reset links, which went to the
/// old address. Returns the account's session version (new if it changed).
pub async fn update_profile(
    pool: &SqlitePool,
    me: &CurrentUser,
    name: &str,
    email: &str,
    language: &str,
) -> Result<i64, sqlx::Error> {
    let email_changes = !email.eq_ignore_ascii_case(&me.email);
    // A transaction: the change and the cancelled links are saved together,
    // or not at all.
    let mut tx = pool.begin().await?;
    let version = sqlx::query_scalar(
        "UPDATE users SET name = ?, email = ?, language = ?, session_version = session_version + ?
         WHERE id = ? RETURNING session_version",
    )
    .bind(name)
    .bind(email)
    .bind(language)
    .bind(i64::from(email_changes))
    .bind(me.id)
    .fetch_one(&mut *tx)
    .await?;
    if email_changes {
        cancel_reset_links(&mut tx, me.id).await?;
    }
    tx.commit().await?;
    Ok(version)
}

/// Set a new password and sign the account out on every device. Older reset
/// links stop working too, so one can't undo the change.
/// Returns the account's new session version.
pub async fn set_password(
    pool: &SqlitePool,
    id: i64,
    password_hash: &str,
) -> Result<i64, sqlx::Error> {
    let mut tx = pool.begin().await?;
    let version = new_password(&mut tx, id, password_hash).await?;
    tx.commit().await?;
    Ok(version)
}

/// Use up a reset link and set the new password in one step, so the same
/// link can't be used twice, even by two requests at the same moment.
/// Returns the account's id, or `None` when the link is unknown or expired.
pub async fn reset_password(
    pool: &SqlitePool,
    token_hash: &str,
    password_hash: &str,
) -> Result<Option<i64>, sqlx::Error> {
    let mut tx = pool.begin().await?;
    let user_id: Option<i64> = sqlx::query_scalar(
        "DELETE FROM password_resets WHERE token_hash = ? AND expires_at > datetime('now')
         RETURNING user_id",
    )
    .bind(token_hash)
    .fetch_optional(&mut *tx)
    .await?;
    let Some(id) = user_id else {
        return Ok(None);
    };
    new_password(&mut tx, id, password_hash).await?;
    tx.commit().await?;
    Ok(Some(id))
}

async fn new_password(
    tx: &mut sqlx::Transaction<'_, sqlx::Sqlite>,
    id: i64,
    password_hash: &str,
) -> Result<i64, sqlx::Error> {
    let version = sqlx::query_scalar(
        "UPDATE users SET password_hash = ?, session_version = session_version + 1
         WHERE id = ? RETURNING session_version",
    )
    .bind(password_hash)
    .bind(id)
    .fetch_one(&mut **tx)
    .await?;
    cancel_reset_links(tx, id).await?;
    Ok(version)
}

async fn cancel_reset_links(
    tx: &mut sqlx::Transaction<'_, sqlx::Sqlite>,
    id: i64,
) -> Result<(), sqlx::Error> {
    sqlx::query("DELETE FROM password_resets WHERE user_id = ?")
        .bind(id)
        .execute(&mut **tx)
        .await
        .map(|_| ())
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
/// False when another sign-in already used it (two at the same moment).
pub async fn set_two_factor_step(pool: &SqlitePool, id: i64, step: u64) -> bool {
    let saved =
        sqlx::query("UPDATE users SET totp_last_step = ? WHERE id = ? AND totp_last_step < ?")
            .bind(step as i64)
            .bind(id)
            .bind(step as i64)
            .execute(pool)
            .await;
    matches!(saved, Ok(r) if r.rows_affected() > 0)
}

/// Turn two-factor login on with `secret` (already confirmed with a code from
/// `step`), replacing any recovery codes with `recovery_hashes`. Other
/// devices are signed out: they signed in without a code.
/// Returns the account's new session version.
pub async fn enable_two_factor(
    pool: &SqlitePool,
    id: i64,
    secret: &[u8],
    step: u64,
    recovery_hashes: &[String],
) -> Result<i64, sqlx::Error> {
    let hex = crate::app::security::to_hex(secret);
    let mut tx = pool.begin().await?;
    let version = sqlx::query_scalar(
        "UPDATE users SET totp_secret = ?, totp_last_step = ?, session_version = session_version + 1
         WHERE id = ? RETURNING session_version",
    )
    .bind(crate::app::secrets::encrypt(&hex))
    .bind(step as i64)
    .bind(id)
    .fetch_one(&mut *tx)
    .await?;
    replace_recovery_codes(&mut tx, id, recovery_hashes).await?;
    tx.commit().await?;
    Ok(version)
}

/// New recovery codes for someone who has two-factor login on. Other devices
/// are signed out, in case one of them signed in with an old code.
/// Returns the account's new session version.
pub async fn set_recovery_codes(
    pool: &SqlitePool,
    id: i64,
    hashes: &[String],
) -> Result<i64, sqlx::Error> {
    let mut tx = pool.begin().await?;
    let version = sqlx::query_scalar(
        "UPDATE users SET session_version = session_version + 1 WHERE id = ?
         RETURNING session_version",
    )
    .bind(id)
    .fetch_one(&mut *tx)
    .await?;
    replace_recovery_codes(&mut tx, id, hashes).await?;
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
