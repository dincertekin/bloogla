//! People: who can sign in and with which role (admins only).
//! Each person's own account is in `profile.rs`.

use super::alert;
use crate::app::models::{CurrentUser, Role};
use crate::app::security::{hash_password, log_event, temporary_password};
use crate::app::state::AppState;
use crate::content::text::escape_html;
use crate::db::{or_log, settings};

use askama::Template;
use axum::extract::{Extension, Path, State};
use axum::http::StatusCode;
use axum::response::{Html, IntoResponse, Response};
use axum_extra::extract::Form;
use serde::Deserialize;

/// One person in the People list.
pub struct UserRow {
    pub id: i64,
    pub name: String,
    pub email: String,
    pub role: Role,
    pub post_count: i64,
}

/// Admin page for managing who can sign in.
#[derive(Template)]
#[template(path = "users.html")]
pub struct UsersTemplate {
    pub blog_name: String,
    pub me: CurrentUser,
    pub users: Vec<UserRow>,
    pub roles: [Role; 3],
}

/// A new row in the People list.
#[derive(Template)]
#[template(path = "user_item.html")]
pub struct UserItemTemplate {
    pub me: CurrentUser,
    pub user: UserRow,
    pub roles: [Role; 3],
}

#[derive(Deserialize)]
pub struct NewUserForm {
    name: String,
    email: String,
    role: String,
}

#[derive(Deserialize)]
pub struct RoleForm {
    role: String,
}

async fn user_rows(state: &AppState) -> Vec<UserRow> {
    let rows: Vec<(i64, String, String, String, i64)> = or_log(
        sqlx::query_as(
            "SELECT u.id, u.name, u.email, u.role,
                    (SELECT COUNT(*) FROM posts p WHERE p.author_id = u.id AND p.is_page = 0)
             FROM users u ORDER BY u.id",
        )
        .fetch_all(&state.pool)
        .await,
        "list users",
    );
    rows.into_iter()
        .map(|(id, name, email, role, post_count)| UserRow {
            id,
            name,
            email,
            role: Role::parse(&role).unwrap_or(Role::Author),
            post_count,
        })
        .collect()
}

/// GET /admin/users -> Everyone who can sign in.
pub async fn page(
    State(state): State<AppState>,
    Extension(me): Extension<CurrentUser>,
) -> impl IntoResponse {
    UsersTemplate {
        blog_name: settings::load(&state.pool).await.blog_name.clone(),
        users: user_rows(&state).await,
        roles: Role::ALL,
        me,
    }
}

/// POST /admin/users -> Add a person with a temporary password to share with them.
pub async fn create(
    State(state): State<AppState>,
    Extension(me): Extension<CurrentUser>,
    Form(form): Form<NewUserForm>,
) -> Response {
    let lang = me.lang;
    let name = form.name.trim();
    let email = form.email.trim();
    let Some(role) = Role::parse(&form.role) else {
        return alert(me.lang, "error", "Choose a role.");
    };
    if !email.contains('@') {
        return alert(me.lang, "error", "Enter a valid email address.");
    }

    let taken: Option<i64> = or_log(
        sqlx::query_scalar("SELECT id FROM users WHERE email = ? COLLATE NOCASE")
            .bind(email)
            .fetch_optional(&state.pool)
            .await,
        "check email",
    );
    if taken.is_some() {
        return alert(
            me.lang,
            "error",
            "Someone with this email can already sign in.",
        );
    }

    let password = temporary_password();
    let hash = match hash_password(&password) {
        Ok(hash) => hash,
        Err(e) => {
            tracing::error!("{e}");
            return alert(
                me.lang,
                "error",
                "Couldn't create the account. Please try again.",
            );
        }
    };
    let id = match sqlx::query(
        "INSERT INTO users (name, email, password_hash, role) VALUES (?, ?, ?, ?)",
    )
    .bind(name)
    .bind(email)
    .bind(&hash)
    .bind(role.as_str())
    .execute(&state.pool)
    .await
    {
        Ok(r) => {
            log_event(
                "user_created",
                &[("user", email), ("role", role.as_str()), ("by", &me.email)],
            );
            r.last_insert_rowid()
        }
        Err(e) => {
            tracing::error!("Failed to create user: {e}");
            return alert(
                me.lang,
                "error",
                "Couldn't create the account. Please try again.",
            );
        }
    };

    let row = UserItemTemplate {
        me,
        user: UserRow {
            id,
            name: name.to_string(),
            email: email.to_string(),
            role,
            post_count: 0,
        },
        roles: Role::ALL,
    };
    let row_html = match row.render() {
        Ok(html) => html,
        Err(e) => {
            tracing::error!("Failed to render user row: {e}");
            String::new()
        }
    };

    let message = lang.fill(
        "{name} can now sign in at {url} with the temporary password {password}. Share it with them privately; they can change it from their profile.",
        &[
            &escape_html(if name.is_empty() { email } else { name }),
            &format!("<code>{}/admin/login</code>", escape_html(&state.config.base_url)),
            &format!("<code>{password}</code>"),
        ],
    );
    Html(format!(
        r#"<div class="alert alert-success">{message}</div>
        <div hx-swap-oob="beforeend:#user-list">{row_html}</div>"#
    ))
    .into_response()
}

/// POST /admin/users/:id/role -> Change someone's role (not your own).
pub async fn update_role(
    State(state): State<AppState>,
    Extension(me): Extension<CurrentUser>,
    Path(id): Path<i64>,
    Form(form): Form<RoleForm>,
) -> Response {
    if id == me.id {
        return alert(me.lang, "error", "You can't change your own role.");
    }
    let Some(role) = Role::parse(&form.role) else {
        return StatusCode::BAD_REQUEST.into_response();
    };
    match sqlx::query("UPDATE users SET role = ? WHERE id = ?")
        .bind(role.as_str())
        .bind(id)
        .execute(&state.pool)
        .await
    {
        Ok(r) if r.rows_affected() > 0 => {
            log_event(
                "role_changed",
                &[
                    ("user_id", &id.to_string()),
                    ("role", role.as_str()),
                    ("by", &me.email),
                ],
            );
            Html(format!(
                r#"<span class="form-hint">{}</span>"#,
                me.t("Saved.")
            ))
            .into_response()
        }
        Ok(_) => StatusCode::NOT_FOUND.into_response(),
        Err(e) => {
            tracing::error!("Failed to change role: {e}");
            StatusCode::INTERNAL_SERVER_ERROR.into_response()
        }
    }
}

/// POST /admin/users/:id/password -> Give someone a new temporary password.
pub async fn reset_password(
    State(state): State<AppState>,
    Extension(me): Extension<CurrentUser>,
    Path(id): Path<i64>,
) -> Response {
    if id == me.id {
        return alert(
            me.lang,
            "error",
            "Change your own password from your profile.",
        );
    }
    let password = temporary_password();
    let Ok(hash) = hash_password(&password) else {
        return StatusCode::INTERNAL_SERVER_ERROR.into_response();
    };
    // Also signs them out everywhere, in case someone else had their password.
    match crate::db::users::set_password(&state.pool, id, &hash).await {
        Ok(_) => {
            log_event(
                "password_reset_by_admin",
                &[("user_id", &id.to_string()), ("by", &me.email)],
            );
            alert(
                me.lang,
                "info",
                &me.lang.tv(
                    "New temporary password: {password}. Share it with them privately.",
                    format!("<code>{password}</code>"),
                ),
            )
        }
        Err(sqlx::Error::RowNotFound) => StatusCode::NOT_FOUND.into_response(),
        Err(e) => {
            tracing::error!("Failed to reset password: {e}");
            StatusCode::INTERNAL_SERVER_ERROR.into_response()
        }
    }
}

/// DELETE /admin/users/:id -> Remove someone; their posts move to you.
pub async fn delete(
    State(state): State<AppState>,
    Extension(me): Extension<CurrentUser>,
    Path(id): Path<i64>,
) -> StatusCode {
    if id == me.id {
        return StatusCode::FORBIDDEN;
    }
    let result = async {
        let mut tx = state.pool.begin().await?;
        sqlx::query("UPDATE posts SET author_id = ? WHERE author_id = ?")
            .bind(me.id)
            .bind(id)
            .execute(&mut *tx)
            .await?;
        let deleted = sqlx::query("DELETE FROM users WHERE id = ?")
            .bind(id)
            .execute(&mut *tx)
            .await?
            .rows_affected();
        tx.commit().await?;
        Ok::<u64, sqlx::Error>(deleted)
    }
    .await;

    match result {
        Ok(0) => StatusCode::NOT_FOUND,
        Ok(_) => {
            log_event(
                "user_deleted",
                &[("user_id", &id.to_string()), ("by", &me.email)],
            );
            StatusCode::OK
        }
        Err(e) => {
            tracing::error!("Failed to delete user: {e}");
            StatusCode::INTERNAL_SERVER_ERROR
        }
    }
}
