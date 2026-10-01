use crate::models::{AppState, LoginForm};
use crate::templates::LoginTemplate;

use argon2::password_hash::{PasswordHash, PasswordVerifier};
use argon2::Argon2;
use askama_axum::IntoResponse;
use axum::extract::{Form, State};
use axum::response::Redirect;
use tower_sessions::Session;

use axum::{
    extract::Request,
    http::{header, Method, StatusCode},
    middleware::Next,
    response::Response,
};

/// Reject cross-site state-changing requests (CSRF protection).
///
/// Browsers send `Sec-Fetch-Site` on every request; anything other than
/// `same-origin` (or `none`, a user-initiated navigation) is refused. Older
/// browsers without it fall back to comparing `Origin` against the configured
/// base URL. Requests with neither header come from non-browser clients, which
/// cannot carry a victim's cookies, so they are allowed through.
pub async fn csrf_guard(State(state): State<AppState>, req: Request, next: Next) -> Response {
    if matches!(*req.method(), Method::GET | Method::HEAD | Method::OPTIONS) {
        return next.run(req).await;
    }

    let headers = req.headers();
    let allowed = match headers.get("sec-fetch-site") {
        Some(site) => site == "same-origin" || site == "none",
        None => match headers.get(header::ORIGIN) {
            Some(origin) => origin.as_bytes() == state.config.base_url.as_bytes(),
            None => true,
        },
    };

    if allowed {
        next.run(req).await
    } else {
        (StatusCode::FORBIDDEN, "Cross-site request blocked").into_response()
    }
}

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
