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

/// Someone's Dashboard layout, as saved (see `handlers/admin/dashboard.rs`).
pub async fn dashboard_layout(pool: &SqlitePool, id: i64) -> String {
    or_log(
        sqlx::query_scalar("SELECT dashboard_layout FROM users WHERE id = ?")
            .bind(id)
            .fetch_optional(pool)
            .await,
        "dashboard layout",
    )
    .unwrap_or_default()
}

/// Save the order and visibility of someone's Dashboard boxes.
pub async fn set_dashboard_layout(
    pool: &SqlitePool,
    id: i64,
    layout: &str,
) -> Result<(), sqlx::Error> {
    sqlx::query("UPDATE users SET dashboard_layout = ? WHERE id = ?")
        .bind(layout)
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

/// Save a new reset link for `user_id` (replacing older ones) that works
/// for `minutes`. Only the hash of the link's code is stored.
pub async fn add_reset_link(
    pool: &SqlitePool,
    user_id: i64,
    token_hash: &str,
    minutes: i64,
) -> Result<(), sqlx::Error> {
    sqlx::query("DELETE FROM password_resets WHERE user_id = ? OR expires_at < datetime('now')")
        .bind(user_id)
        .execute(pool)
        .await?;
    sqlx::query(
        "INSERT INTO password_resets (token_hash, user_id, expires_at)
         VALUES (?, ?, datetime('now', ?))",
    )
    .bind(token_hash)
    .bind(user_id)
    .bind(format!("+{minutes} minutes"))
    .execute(pool)
    .await?;
    Ok(())
}

/// The account a reset link belongs to, while the link still works.
pub async fn reset_link_owner(pool: &SqlitePool, token_hash: &str) -> Option<i64> {
    or_log(
        sqlx::query_scalar(
            "SELECT user_id FROM password_resets
             WHERE token_hash = ? AND expires_at > datetime('now')",
        )
        .bind(token_hash)
        .fetch_optional(pool)
        .await,
        "check password reset link",
    )
}

// ---- People (Admin → People) ----

/// Someone in the People list.
#[derive(sqlx::FromRow)]
pub struct Person {
    pub id: i64,
    pub name: String,
    pub email: String,
    pub role: String,
    /// Posts they wrote that aren't in the trash.
    pub post_count: i64,
    pub two_factor: bool,
}

/// Everyone who can sign in, oldest account first.
pub async fn people(pool: &SqlitePool) -> Vec<Person> {
    or_log(
        sqlx::query_as(
            "SELECT u.id, u.name, u.email, u.role,
                    (SELECT COUNT(*) FROM posts p
                     WHERE p.author_id = u.id AND p.is_page = 0 AND p.trashed_at IS NULL)
                        AS post_count,
                    u.totp_secret IS NOT NULL AS two_factor
             FROM users u ORDER BY u.id",
        )
        .fetch_all(pool)
        .await,
        "list users",
    )
}

/// How many accounts exist (none until setup is finished).
pub async fn count(pool: &SqlitePool) -> Result<i64, sqlx::Error> {
    sqlx::query_scalar("SELECT COUNT(*) FROM users")
        .fetch_one(pool)
        .await
}

/// The account that signs in with this email (upper or lower case alike).
pub async fn id_by_email(pool: &SqlitePool, email: &str) -> Option<i64> {
    or_log(
        sqlx::query_scalar("SELECT id FROM users WHERE email = ? COLLATE NOCASE")
            .bind(email)
            .fetch_optional(pool)
            .await,
        "find account by email",
    )
}

/// What signing in needs to know about the account with this email:
/// `(id, password hash, session version, two-factor on)`.
pub async fn for_sign_in(pool: &SqlitePool, email: &str) -> Option<(i64, String, i64, bool)> {
    or_log(
        sqlx::query_as(
            "SELECT id, password_hash, session_version, totp_secret IS NOT NULL
             FROM users WHERE email = ? COLLATE NOCASE",
        )
        .bind(email)
        .fetch_optional(pool)
        .await,
        "look up login",
    )
}

/// Someone's email and session version (it changes when they're signed out
/// everywhere).
pub async fn email_and_version(pool: &SqlitePool, id: i64) -> Option<(String, i64)> {
    or_log(
        sqlx::query_as("SELECT email, session_version FROM users WHERE id = ?")
            .bind(id)
            .fetch_optional(pool)
            .await,
        "load account",
    )
}

/// The emails of admins and editors: the people who hear about new comments
/// and messages.
pub async fn editor_emails(pool: &SqlitePool) -> Vec<String> {
    or_log(
        sqlx::query_scalar("SELECT email FROM users WHERE role IN ('admin', 'editor')")
            .fetch_all(pool)
            .await,
        "load editors' emails",
    )
}

/// The name of the person who signs in with this email.
pub async fn name_by_email(pool: &SqlitePool, email: &str) -> Option<String> {
    or_log(
        sqlx::query_scalar("SELECT name FROM users WHERE email = ? COLLATE NOCASE")
            .bind(email)
            .fetch_optional(pool)
            .await,
        "find account name by email",
    )
}

/// The site owner: the account made during setup.
pub async fn owner_id(pool: &SqlitePool) -> Option<i64> {
    or_log(
        sqlx::query_scalar("SELECT id FROM users ORDER BY id ASC LIMIT 1")
            .fetch_optional(pool)
            .await,
        "find site owner",
    )
}

/// The email someone signs in with.
pub async fn email_of(pool: &SqlitePool, id: i64) -> String {
    or_log(
        sqlx::query_scalar("SELECT email FROM users WHERE id = ?")
            .bind(id)
            .fetch_optional(pool)
            .await,
        "load account email",
    )
    .unwrap_or_default()
}

/// The language someone picked for the admin, or "" for the site's.
pub async fn language_of(pool: &SqlitePool, id: i64) -> String {
    or_log(
        sqlx::query_scalar("SELECT language FROM users WHERE id = ?")
            .bind(id)
            .fetch_optional(pool)
            .await,
        "load language",
    )
    .unwrap_or_default()
}

/// The stored password hash, to check a password someone typed.
pub async fn password_hash_of(pool: &SqlitePool, id: i64) -> Option<String> {
    or_log(
        sqlx::query_scalar("SELECT password_hash FROM users WHERE id = ?")
            .bind(id)
            .fetch_optional(pool)
            .await,
        "load password",
    )
}

/// Finish setup: add the site's first account (an admin) and its first
/// settings together, and return the account's id. `None` when the site
/// already has an account: checked inside the same step, so two setups at
/// the same moment can't both succeed.
pub async fn create_owner(
    pool: &SqlitePool,
    name: &str,
    email: &str,
    password_hash: &str,
    first_settings: &[(&str, &str)],
) -> Result<Option<i64>, sqlx::Error> {
    let mut tx = pool.begin().await?;
    let accounts: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM users")
        .fetch_one(&mut *tx)
        .await?;
    if accounts > 0 {
        return Ok(None);
    }
    let user_id = sqlx::query(
        "INSERT INTO users (name, email, password_hash, role) VALUES (?, ?, ?, 'admin')",
    )
    .bind(name)
    .bind(email)
    .bind(password_hash)
    .execute(&mut *tx)
    .await?
    .last_insert_rowid();
    for (key, value) in first_settings {
        sqlx::query(
            "INSERT INTO settings (key, value) VALUES (?, ?)
             ON CONFLICT(key) DO UPDATE SET value = excluded.value",
        )
        .bind(key)
        .bind(value)
        .execute(&mut *tx)
        .await?;
    }
    tx.commit().await?;
    crate::db::settings::invalidate();
    Ok(Some(user_id))
}

/// Sign out everyone, on every device (for the command line, when the
/// server may not be running). The sessions table only exists once the
/// server has run, so a missing table is fine.
pub async fn forget_all_sessions(pool: &SqlitePool) {
    let _ = sqlx::query("DELETE FROM tower_sessions")
        .execute(pool)
        .await;
}

/// Add an account and return its id.
pub async fn create(
    pool: &SqlitePool,
    name: &str,
    email: &str,
    password_hash: &str,
    role: Role,
) -> Result<i64, sqlx::Error> {
    let result =
        sqlx::query("INSERT INTO users (name, email, password_hash, role) VALUES (?, ?, ?, ?)")
            .bind(name)
            .bind(email)
            .bind(password_hash)
            .bind(role.as_str())
            .execute(pool)
            .await?;
    Ok(result.last_insert_rowid())
}

/// Change what someone may do. `false` when there is no such account.
pub async fn set_role(pool: &SqlitePool, id: i64, role: Role) -> Result<bool, sqlx::Error> {
    let result = sqlx::query("UPDATE users SET role = ? WHERE id = ?")
        .bind(role.as_str())
        .bind(id)
        .execute(pool)
        .await?;
    Ok(result.rows_affected() > 0)
}

/// Remove an account; its posts move to `new_author`, so nothing is lost.
/// `false` when there is no such account.
pub async fn delete(pool: &SqlitePool, id: i64, new_author: i64) -> Result<bool, sqlx::Error> {
    let mut tx = pool.begin().await?;
    sqlx::query("UPDATE posts SET author_id = ? WHERE author_id = ?")
        .bind(new_author)
        .bind(id)
        .execute(&mut *tx)
        .await?;
    let deleted = sqlx::query("DELETE FROM users WHERE id = ?")
        .bind(id)
        .execute(&mut *tx)
        .await?
        .rows_affected();
    tx.commit().await?;
    Ok(deleted > 0)
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
