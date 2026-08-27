use crate::models::{AppState, LoginForm};
use crate::templates::LoginTemplate;

use argon2::password_hash::{PasswordHash, PasswordVerifier};
use argon2::Argon2;
use askama_axum::IntoResponse;
use axum::extract::{Form, State};
use axum::response::Redirect;
use tower_sessions::Session;

use axum::{extract::Request, http::StatusCode, middleware::Next, response::Response};

pub async fn auth_middleware(session: Session, req: Request, next: Next) -> Response {
    let logged_in: Option<bool> = session.get("admin_logged_in").await.unwrap_or(None);

    if logged_in == Some(true) {
        next.run(req).await
    } else {
        // If it's a standard page GET request, redirect to login.
        // For HTMX/POST/DELETE API requests, return 401 Unauthorized.
        if req.method() == axum::http::Method::GET {
            Redirect::to("/admin/login").into_response()
        } else {
            StatusCode::UNAUTHORIZED.into_response()
        }
    }
}

/// GET /admin/login -> Show login form.
pub async fn login_page() -> impl IntoResponse {
    LoginTemplate { error: None }
}

/// POST /admin/login -> Check the password user entered with Argon2 hash.
pub async fn handle_login(
    State(state): State<AppState>,
    session: Session,
    Form(form): Form<LoginForm>,
) -> impl IntoResponse {
    let user = sqlx::query_scalar::<_, String>("SELECT password_hash FROM users WHERE email = ?")
        .bind(&form.email)
        .fetch_optional(&state.pool)
        .await
        .ok()
        .flatten();

    if let Some(stored_hash) = user {
        if let Ok(parsed_hash) = PasswordHash::new(&stored_hash) {
            if Argon2::default()
                .verify_password(form.password.as_bytes(), &parsed_hash)
                .is_ok()
            {
                if let Err(e) = session.cycle_id().await {
                    eprintln!("Failed to cycle session id: {e}");
                }
                let _ = session.insert("admin_logged_in", true).await;
                return Redirect::to("/admin").into_response();
            }
        }
    }

    LoginTemplate {
        error: Some("Invalid email or password.".into()),
    }
    .into_response()
}

/// GET /admin/logout -> Reset current session and redirect to login page.
pub async fn handle_logout(session: Session) -> impl IntoResponse {
    let _ = session.flush().await;
    Redirect::to("/admin/login")
}
