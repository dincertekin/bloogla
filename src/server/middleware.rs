//! Middleware: code that runs before (and after) request handlers.
//!
//! The order they run in is set in `routes.rs`.

use crate::app::models::{CurrentUser, SESSION_USER_ID, SESSION_VERSION};
use crate::app::state::AppState;
use crate::handlers::admin::forbidden;

use axum::body::Body;
use axum::extract::{ConnectInfo, Request, State};
use axum::http::{header, HeaderValue, Method, StatusCode};
use axum::middleware::Next;
use axum::response::{IntoResponse, Redirect, Response};
use std::net::SocketAddr;
use std::sync::atomic::Ordering;
use tower_sessions::Session;

/// Largest page the ETag middleware will buffer.
const MAX_ETAG_BODY: usize = 16 * 1024 * 1024;

/// One log line per page request: method, path, status and time taken.
/// Static files and health checks are skipped to keep logs readable.
pub async fn access_log(req: Request, next: Next) -> Response {
    let method = req.method().clone();
    let path = req.uri().path().to_string();
    let quiet = ["/static/", "/uploads/", "/theme-assets/", "/health"]
        .iter()
        .any(|prefix| path.starts_with(prefix));

    let started = std::time::Instant::now();
    let response = next.run(req).await;
    if !quiet {
        let status = response.status().as_u16();
        let ms = (started.elapsed().as_secs_f64() * 10_000.0).round() / 10.0;
        if status >= 500 {
            tracing::error!(%method, %path, status, ms, "request");
        } else {
            tracing::info!(%method, %path, status, ms, "request");
        }
    }
    response
}

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

/// While setup is pending, send every page to `/setup`.
pub async fn setup_guard(State(state): State<AppState>, req: Request, next: Next) -> Response {
    let path = req.uri().path();
    let exempt = path.starts_with("/setup") || path.starts_with("/static/") || path == "/health";

    if !exempt && state.setup_pending.load(Ordering::Acquire) {
        return Redirect::to("/setup").into_response();
    }
    next.run(req).await
}

/// Admin screens: load the signed-in person into the request, or send the
/// visitor to the login page.
pub async fn require_login(
    State(state): State<AppState>,
    session: Session,
    mut req: Request,
    next: Next,
) -> Response {
    let user_id: Option<i64> = session.get(SESSION_USER_ID).await.unwrap_or(None);
    let user = match user_id {
        Some(id) => crate::db::users::find(&state.pool, id).await,
        None => None,
    };
    // Sessions from before a password change no longer count.
    let session_version: i64 = session
        .get(SESSION_VERSION)
        .await
        .unwrap_or(None)
        .unwrap_or(0);
    let user = match user {
        Some(user) if user.session_version == session_version => Some(user),
        Some(user) => {
            crate::app::security::log_event(
                "session_expired",
                &[("user", &user.email), ("reason", "password changed")],
            );
            let _ = session.flush().await;
            None
        }
        None => None,
    };

    match user {
        Some(user) => {
            req.extensions_mut().insert(user);
            next.run(req).await
        }
        // Page loads go to the login form; HTMX and form requests get 401.
        None if req.method() == Method::GET => Redirect::to("/admin/login").into_response(),
        None => StatusCode::UNAUTHORIZED.into_response(),
    }
}

/// Only admins may continue (settings, people, subscribers).
/// Runs after [`require_login`].
pub async fn require_admin(req: Request, next: Next) -> Response {
    match req.extensions().get::<CurrentUser>() {
        Some(user) if user.is_admin() => next.run(req).await,
        user => forbidden(user.map(|u| u.lang).unwrap_or_default()),
    }
}

/// Only admins and editors may continue (pages, tags, comments, deleting media).
/// Runs after [`require_login`].
pub async fn require_editor(req: Request, next: Next) -> Response {
    match req.extensions().get::<CurrentUser>() {
        Some(user) if user.can_edit_all() => next.run(req).await,
        user => forbidden(user.map(|u| u.lang).unwrap_or_default()),
    }
}

/// JSON API writes: require `Authorization: Bearer bl_...` and act as the
/// token's owner.
pub async fn require_api_token(
    State(state): State<AppState>,
    mut req: Request,
    next: Next,
) -> Response {
    let token = req
        .headers()
        .get(header::AUTHORIZATION)
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.strip_prefix("Bearer "))
        .map(str::trim)
        .filter(|t| t.starts_with("bl_"));
    let Some(token) = token else {
        return crate::handlers::api::error(
            StatusCode::UNAUTHORIZED,
            "Send an API token as 'Authorization: Bearer bl_...'. Create one on your Profile page.",
        );
    };

    let token_hash = crate::app::security::hash_token(token);
    let Some(user) = crate::db::users::find_by_token_hash(&state.pool, &token_hash).await else {
        let ip = req
            .extensions()
            .get::<ConnectInfo<SocketAddr>>()
            .map(|ConnectInfo(peer)| {
                crate::handlers::client_ip(&state, *peer, req.headers()).to_string()
            })
            .unwrap_or_default();
        crate::app::security::log_event("api_token_rejected", &[("ip", &ip)]);
        return crate::handlers::api::error(StatusCode::UNAUTHORIZED, "This API token isn't valid");
    };
    req.extensions_mut().insert(user);
    next.run(req).await
}

/// Theme folders also hold templates; only their `static/` files are public.
///
/// Themes link their files with the theme's version (`style.css?v=2.0.0`), so
/// those URLs change on every theme update and browsers may keep them for a
/// year. Without a version, browsers check for changes each time.
pub async fn theme_static_only(req: Request, next: Next) -> Response {
    // Inside `/theme-assets`, so the path looks like `/<theme>/static/...`.
    let mut segments = req.uri().path().trim_start_matches('/').split('/');
    let theme = segments.next().unwrap_or_default();
    // Hidden folders are unfinished theme uploads.
    if theme.is_empty() || theme.starts_with('.') || segments.next() != Some("static") {
        return StatusCode::NOT_FOUND.into_response();
    }
    let versioned = req
        .uri()
        .query()
        .is_some_and(|q| q.split('&').any(|pair| pair.starts_with("v=")));

    let mut response = next.run(req).await;
    if response.status().is_success() {
        let cache = if versioned {
            "public, max-age=31536000, immutable"
        } else {
            "no-cache"
        };
        response
            .headers_mut()
            .insert(header::CACHE_CONTROL, HeaderValue::from_static(cache));
    }
    response
}

/// Add an `ETag` to public HTML and XML responses and answer `304 Not Modified`
/// when the browser or feed reader already has the same version.
pub async fn etag(req: Request, next: Next) -> Response {
    use std::hash::{Hash, Hasher};

    let path = req.uri().path();
    let eligible =
        req.method() == Method::GET && !path.starts_with("/admin") && !path.starts_with("/setup");
    let if_none_match = req.headers().get(header::IF_NONE_MATCH).cloned();

    let response = next.run(req).await;
    let is_page = response
        .headers()
        .get(header::CONTENT_TYPE)
        .and_then(|v| v.to_str().ok())
        .is_some_and(|ct| ct.starts_with("text/html") || ct.contains("xml"));
    if !eligible || response.status() != StatusCode::OK || !is_page {
        return response;
    }

    let (mut parts, body) = response.into_parts();
    let Ok(bytes) = axum::body::to_bytes(body, MAX_ETAG_BODY).await else {
        return StatusCode::INTERNAL_SERVER_ERROR.into_response();
    };

    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    bytes.hash(&mut hasher);
    let etag = format!("W/\"{:016x}\"", hasher.finish());

    if let Ok(value) = HeaderValue::from_str(&etag) {
        parts.headers.insert(header::ETAG, value);
    }
    // Always revalidate, so new posts show up immediately.
    parts
        .headers
        .insert(header::CACHE_CONTROL, HeaderValue::from_static("no-cache"));

    if if_none_match.is_some_and(|v| v.as_bytes() == etag.as_bytes()) {
        parts.status = StatusCode::NOT_MODIFIED;
        parts.headers.remove(header::CONTENT_LENGTH);
        return Response::from_parts(parts, Body::empty());
    }
    Response::from_parts(parts, Body::from(bytes))
}

/// A bug made a request crash (panic): log it and answer with a plain 500
/// page instead of dropping the connection. The server keeps running.
pub fn crash_response(details: Box<dyn std::any::Any + Send + 'static>) -> Response {
    let message = details
        .downcast_ref::<String>()
        .map(String::as_str)
        .or_else(|| details.downcast_ref::<&str>().copied())
        .unwrap_or("unknown error");
    tracing::error!("Request crashed: {message}");
    (
        StatusCode::INTERNAL_SERVER_ERROR,
        axum::response::Html(
            "<h1>Something went wrong</h1><p>Please try again. If it keeps happening, \
             the site's owner can find details in the server log.</p>",
        ),
    )
        .into_response()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn crashes_become_a_plain_500_page() {
        let response = crash_response(Box::new("boom"));
        assert_eq!(response.status(), StatusCode::INTERNAL_SERVER_ERROR);
    }
}
