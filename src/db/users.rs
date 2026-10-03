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
