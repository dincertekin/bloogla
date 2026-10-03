//! Your own account: name, email, language, password and API tokens.

use super::alert;
use crate::app::models::CurrentUser;
use crate::app::security::{
    hash_password, hash_token, log_event, missing_password_rules, new_api_token, verify_password,
    PasswordRule, PASSWORD_RULES,
};
use crate::app::state::AppState;
use crate::content::text::{display_date, display_datetime};
use crate::db::{or_log, settings};
use crate::i18n::Lang;

use askama::Template;
use axum::extract::{Extension, Path, State};
use axum::http::StatusCode;
use axum::response::{Html, IntoResponse, Response};
use axum_extra::extract::Form;
use serde::Deserialize;
use tower_sessions::Session;

/// The signed-in person's own name, email and password.
#[derive(Template)]
#[template(path = "profile.html")]
pub struct ProfileTemplate {
    pub blog_name: String,
    pub me: CurrentUser,
    /// The person's own language choice (`""` = the site's).
    pub language: String,
    pub languages: Vec<Lang>,
    pub base_url: String,
    pub tokens: Vec<TokenRow>,
    /// The password checklist.
    pub password_rules: &'static [PasswordRule],
}

/// One of your API tokens (the token itself is never shown again).
pub struct TokenRow {
    pub id: i64,
    pub name: String,
    pub created: String,
    pub last_used: Option<String>,
}

/// A newly created token row.
#[derive(Template)]
#[template(path = "token_item.html")]
pub struct TokenItemTemplate {
    pub me: CurrentUser,
    pub token: TokenRow,
}

#[derive(Deserialize)]
pub struct ProfileForm {
    name: String,
    email: String,
    /// `en`, `tr`, or empty for the site's language.
    #[serde(default)]
    language: String,
}

#[derive(Deserialize)]
pub struct PasswordForm {
    current_password: String,
    new_password: String,
    confirm_password: String,
}

/// GET /admin/profile -> Your own name, email and password.
pub async fn page(
    State(state): State<AppState>,
    Extension(me): Extension<CurrentUser>,
) -> impl IntoResponse {
    let rows: Vec<(i64, String, String, Option<String>)> = or_log(
        sqlx::query_as(
            "SELECT id, name, created_at, last_used_at FROM api_tokens
             WHERE user_id = ? ORDER BY id DESC",
        )
        .bind(me.id)
        .fetch_all(&state.pool)
        .await,
        "list tokens",
    );
    let language: String = or_log(
        sqlx::query_scalar("SELECT language FROM users WHERE id = ?")
            .bind(me.id)
            .fetch_one(&state.pool)
            .await,
        "load language",
    );
    ProfileTemplate {
        blog_name: settings::load(&state.pool).await.blog_name.clone(),
        language,
        languages: Lang::all(),
        base_url: state.config.base_url.clone(),
        password_rules: PASSWORD_RULES,
        tokens: rows
            .into_iter()
            .map(|row| token_row(me.lang, row))
            .collect(),
        me,
    }
}

fn token_row(
    lang: Lang,
    (id, name, created, last_used): (i64, String, String, Option<String>),
) -> TokenRow {
    TokenRow {
        id,
        name,
        created: display_date(lang, &created),
        last_used: last_used.map(|t| display_datetime(lang, &t)),
    }
}

#[derive(Deserialize)]
pub struct TokenForm {
    name: String,
}

/// POST /admin/profile/tokens -> Create an API token and show it once.
pub async fn create_token(
    State(state): State<AppState>,
    Extension(me): Extension<CurrentUser>,
    Form(form): Form<TokenForm>,
) -> Response {
    let name = form.name.trim();
    let name = if name.is_empty() { "API token" } else { name };
    let token = new_api_token();

    let id =
        match sqlx::query("INSERT INTO api_tokens (user_id, name, token_hash) VALUES (?, ?, ?)")
            .bind(me.id)
            .bind(name)
            .bind(hash_token(&token))
            .execute(&state.pool)
            .await
        {
            Ok(r) => {
                log_event("api_token_created", &[("user", &me.email), ("name", name)]);
                r.last_insert_rowid()
            }
            Err(e) => {
                tracing::error!("Failed to create token: {e}");
                return alert(
                    me.lang,
                    "error",
                    "Couldn't create the token. Please try again.",
                );
            }
        };

    let row = TokenItemTemplate {
        me: me.clone(),
        token: TokenRow {
            id,
            name: name.to_string(),
            created: me.t("Today").to_string(),
            last_used: None,
        },
    };
    let row_html = row.render().unwrap_or_default();
    Html(format!(
        r#"<div class="alert alert-success">{}
        <code style="user-select: all">{token}</code></div>
        <div hx-swap-oob="afterbegin:#token-list">{row_html}</div>"#,
        me.t("Copy this token now; it won't be shown again:")
    ))
    .into_response()
}

/// DELETE /admin/profile/tokens/:id -> Revoke one of your tokens.
pub async fn revoke_token(
    State(state): State<AppState>,
    Extension(me): Extension<CurrentUser>,
    Path(id): Path<i64>,
) -> StatusCode {
    match sqlx::query("DELETE FROM api_tokens WHERE id = ? AND user_id = ?")
        .bind(id)
        .bind(me.id)
        .execute(&state.pool)
        .await
    {
        Ok(r) if r.rows_affected() > 0 => {
            log_event(
                "api_token_revoked",
                &[("user", &me.email), ("id", &id.to_string())],
            );
            StatusCode::OK
        }
        Ok(_) => StatusCode::NOT_FOUND,
        Err(e) => {
            tracing::error!("Failed to revoke token: {e}");
            StatusCode::INTERNAL_SERVER_ERROR
        }
    }
}

/// POST /admin/profile -> Update your name and email.
pub async fn update(
    State(state): State<AppState>,
    Extension(me): Extension<CurrentUser>,
    Form(form): Form<ProfileForm>,
) -> Response {
    let email = form.email.trim();
    if !email.contains('@') {
        return alert(me.lang, "error", "Enter a valid email address.");
    }
    let taken: Option<i64> = or_log(
        sqlx::query_scalar("SELECT id FROM users WHERE email = ? COLLATE NOCASE AND id != ?")
            .bind(email)
            .bind(me.id)
            .fetch_optional(&state.pool)
            .await,
        "check email",
    );
    if taken.is_some() {
        return alert(me.lang, "error", "Someone else already uses this email.");
    }

    let language = Lang::parse(&form.language).map_or("", Lang::code);
    let previous_language: String = or_log(
        sqlx::query_scalar("SELECT language FROM users WHERE id = ?")
            .bind(me.id)
            .fetch_one(&state.pool)
            .await,
        "load language",
    );
    match sqlx::query("UPDATE users SET name = ?, email = ?, language = ? WHERE id = ?")
        .bind(form.name.trim())
        .bind(email)
        .bind(language)
        .bind(me.id)
        .execute(&state.pool)
        .await
    {
        // A new language applies to the whole page, so reload it.
        Ok(_) if language != previous_language => (
            [("HX-Refresh", "true")],
            alert(me.lang, "success", "Saved."),
        )
            .into_response(),
        Ok(_) => alert(me.lang, "success", "Saved."),
        Err(e) => {
            tracing::error!("Failed to update profile: {e}");
            alert(
                me.lang,
                "error",
                "Couldn't save your changes. Please try again.",
            )
        }
    }
}

/// POST /admin/profile/password -> Change your own password.
pub async fn update_password(
    State(state): State<AppState>,
    Extension(me): Extension<CurrentUser>,
    session: Session,
    Form(form): Form<PasswordForm>,
) -> Response {
    let missing = missing_password_rules(&form.new_password);
    if !missing.is_empty() {
        let items: String = missing
            .iter()
            .map(|rule| format!("<li>{}</li>", me.lang.t_owned(rule.text)))
            .collect();
        return Html(format!(
            r#"<div class="alert alert-error">{}<ul class="password-missing">{items}</ul></div>"#,
            me.t("Your password still needs:")
        ))
        .into_response();
    }
    if form.new_password != form.confirm_password {
        return alert(me.lang, "error", "The new passwords don't match.");
    }

    let stored: Option<String> = or_log(
        sqlx::query_scalar("SELECT password_hash FROM users WHERE id = ?")
            .bind(me.id)
            .fetch_optional(&state.pool)
            .await,
        "load password",
    );
    // Password hashing is slow on purpose, so it runs off the async threads.
    let current_password = form.current_password.clone();
    let current_ok =
        tokio::task::spawn_blocking(move || verify_password(&current_password, stored.as_deref()))
            .await
            .unwrap_or(false);
    if !current_ok {
        log_event("password_change_failed", &[("user", &me.email)]);
        return alert(me.lang, "error", "Your current password isn't right.");
    }

    let Ok(hash) = hash_password(&form.new_password) else {
        return alert(
            me.lang,
            "error",
            "Couldn't change your password. Please try again.",
        );
    };
    match crate::db::users::set_password(&state.pool, me.id, &hash).await {
        Ok(new_version) => {
            log_event("password_changed", &[("user", &me.email)]);
            // Every other device is now signed out; this one stays signed in.
            crate::handlers::admin::auth::sign_in(&session, me.id, new_version).await;
            alert(
                me.lang,
                "success",
                "Password updated. Other devices have been signed out.",
            )
        }
        Err(e) => {
            tracing::error!("Failed to update password: {e}");
            alert(
                me.lang,
                "error",
                "Couldn't change your password. Please try again.",
            )
        }
    }
}
