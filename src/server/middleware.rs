//! Middleware: code that runs before (and after) request handlers.
//!
//! The order they run in is set in `routes.rs`.

use crate::app::models::{CurrentUser, SESSION_USER_ID, SESSION_VERSION};
use crate::app::state::AppState;
use crate::handlers::admin::forbidden;

use axum::body::Body;
use axum::extract::{Request, State};
use axum::http::{header, HeaderValue, Method, StatusCode};
use axum::middleware::Next;
use axum::response::{IntoResponse, Redirect, Response};
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
    // Sessions from before a change to how the account signs in (password,
    // email, two-factor login) no longer count.
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
                &[("user", &user.email), ("reason", "account sign-in changed")],
            );
            let _ = session.flush().await;
            None
        }
        None => None,
    };

    match user {
        Some(mut user) => {
            // Saving (POST, DELETE) only answers with a message, without the menu.
            if req.method() == Method::GET {
                user.badges = crate::app::models::Badges::load(&state, &user).await;
            }
            req.extensions_mut().insert(user);
            next.run(req).await
        }
        // Page loads go to the login form; HTMX and form requests get 401.
        None if req.method() == Method::GET => Redirect::to("/admin/login").into_response(),
        None => StatusCode::UNAUTHORIZED.into_response(),
    }
}

/// Only admins may continue (settings, themes, people).
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

/// Uploaded JPEG and PNG pictures have a smaller WebP copy next to them
/// (`photo.jpg.webp`, see `handlers/admin/media.rs`). A browser that accepts
/// WebP (all current ones) asking for `/uploads/photo.jpg` gets the copy; any
/// other gets the original. Covers, pictures in posts and copied links all
/// benefit, in every theme, without changing any template.
pub async fn prefer_webp(mut req: Request, next: Next) -> Response {
    // Inside `/uploads`, so the path is just `/<file name>`.
    let name = req.uri().path().trim_start_matches('/').to_string();
    let lower = name.to_ascii_lowercase();
    let has_copy = lower.ends_with(".jpg") || lower.ends_with(".jpeg") || lower.ends_with(".png");
    if !has_copy {
        return next.run(req).await;
    }
    let accepts_webp = req
        .headers()
        .get(header::ACCEPT)
        .and_then(|value| value.to_str().ok())
        .is_some_and(|accept| accept.contains("image/webp"));
    // Upload names are random letters and digits: anything else is never
    // looked up on disk.
    let plain_name = name
        .chars()
        .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '.')
        && !name.contains("..");
    let copy = format!("{name}.webp");
    let copy_exists = plain_name
        && tokio::fs::try_exists(
            std::path::Path::new(crate::handlers::admin::media::UPLOAD_DIR).join(&copy),
        )
        .await
        .unwrap_or(false);
    if accepts_webp && copy_exists {
        if let Ok(uri) = format!("/{copy}").parse() {
            *req.uri_mut() = uri;
        }
    }
    let mut response = next.run(req).await;
    // Caches keep the WebP and the original apart.
    response
        .headers_mut()
        .insert(header::VARY, HeaderValue::from_static("Accept"));
    response
}

/// Theme folders also hold templates; only their `static/` files are public.
///
/// Themes link their files with the theme's version (`style.css?v=2.0.0`), so
/// those URLs change on every theme update and browsers may keep them for a
/// year. Files without a version are mostly ones a stylesheet points to
/// (fonts, small pictures), which rarely change: browsers keep them for a day
/// instead of asking the server about them again on every page.
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
            "public, max-age=86400"
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
    // Pages that must not be kept get no ETag.
    let no_store = response
        .headers()
        .get(header::CACHE_CONTROL)
        .is_some_and(|v| v.as_bytes().ends_with(b"no-store"));
    if !eligible || response.status() != StatusCode::OK || !is_page || no_store {
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
    let page = crate::handlers::plain_page(
        crate::i18n::Lang::default(),
        "Something went wrong",
        "Please try again. If it keeps happening, the site's owner can find details in the server log.",
        Some(("/", "Go to the home page")),
    );
    (StatusCode::INTERNAL_SERVER_ERROR, page).into_response()
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
