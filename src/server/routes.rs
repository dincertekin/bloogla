//! Every URL Bloogla answers, and the handler function behind it.
//!
//! To add a page: write a handler in `src/handlers/`, then add a `.route(...)`
//! line in the right group below. Admin routes are grouped by the least role
//! that may use them.

use super::{assets, middleware};
use crate::app::state::AppState;
use crate::content::text::url_encode;
use crate::handlers::{admin, api, site};

use axum::extract::DefaultBodyLimit;
use axum::http::{header, HeaderName, HeaderValue};
use axum::middleware::{from_fn, from_fn_with_state};
use axum::routing::{any, delete, get, post, put, MethodRouter};
use axum::Router;
use std::sync::Arc;
use tower_governor::governor::GovernorConfigBuilder;
use tower_governor::key_extractor::SmartIpKeyExtractor;
use tower_governor::GovernorLayer;
use tower_http::catch_panic::CatchPanicLayer;
use tower_http::compression::CompressionLayer;
use tower_http::services::ServeDir;
use tower_http::set_header::SetResponseHeaderLayer;
use tower_sessions::{SessionManagerLayer, SessionStore};

/// Content Security Policy for the admin panel, login and setup: the
/// browser only runs scripts from Bloogla's own files, never inline code or
/// `eval`, so even if someone slipped HTML into a page it couldn't run.
/// Images, audio and video may come from any HTTPS site (posts link to them),
/// and the editor's preview may show YouTube and Vimeo embeds.
const ADMIN_CSP: &str = "default-src 'self'; script-src 'self'; style-src 'self' 'unsafe-inline'; \
    img-src 'self' https: data:; media-src 'self' https:; \
    frame-src https://www.youtube-nocookie.com https://player.vimeo.com; \
    connect-src 'self'; font-src 'self'; object-src 'none'; base-uri 'self'; \
    form-action 'self'; frame-ancestors 'none'";

/// Public URL path of a post (`/post/slug`) or page (`/slug`), percent-encoded
/// so non-Latin slugs are valid in links, redirects, sitemaps and feeds.
pub fn post_path(slug: &str, is_page: bool) -> String {
    let slug = url_encode(slug);
    if is_page {
        format!("/{slug}")
    } else {
        format!("/post/{slug}")
    }
}

/// Build the app: all routes plus the middleware every request passes through.
pub fn build<S: SessionStore + Clone>(
    state: AppState,
    sessions: SessionManagerLayer<S>,
) -> Result<Router, String> {
    let app = Router::new()
        .merge(public_routes())
        .merge(admin_routes(&state)?)
        .merge(api_routes(&state))
        .fallback(site::pages::fallback);

    // Layers wrap everything added before them, so the last one listed runs
    // first: access log → crash guard → sessions → compression → CSRF → setup
    // → ETag → headers.
    let app = with_security_headers(app, state.config.production)
        .layer(from_fn(middleware::etag))
        .layer(from_fn_with_state(state.clone(), middleware::setup_guard))
        .layer(from_fn_with_state(state.clone(), middleware::csrf_guard))
        .layer(CompressionLayer::new())
        .layer(sessions)
        .layer(CatchPanicLayer::custom(middleware::crash_response))
        .layer(from_fn(middleware::access_log))
        .with_state(state);
    Ok(app)
}

/// The public website, feeds, and files.
fn public_routes() -> Router<AppState> {
    // Uploads have random, never-reused names, so they can be cached forever.
    let uploads = Router::new()
        .fallback_service(ServeDir::new(admin::media::UPLOAD_DIR))
        .layer(SetResponseHeaderLayer::overriding(
            header::CACHE_CONTROL,
            HeaderValue::from_static("public, max-age=31536000, immutable"),
        ));
    let theme_assets = Router::new()
        .fallback_service(ServeDir::new("themes"))
        .layer(from_fn(middleware::theme_static_only));

    Router::new()
        .route("/", get(site::pages::home))
        .route("/search", get(site::pages::search))
        .route("/post/:slug", get(site::pages::show_post))
        .route(
            "/post/:slug/comments",
            post(site::comments::submit).layer(DefaultBodyLimit::max(64 * 1024)),
        )
        .route("/tag/:slug", get(site::pages::tag))
        .route(
            "/subscribe",
            post(site::newsletter::subscribe).layer(DefaultBodyLimit::max(8 * 1024)),
        )
        .route("/subscribe/confirm", get(site::newsletter::confirm))
        .route(
            "/unsubscribe",
            get(site::newsletter::unsubscribe_page).post(site::newsletter::unsubscribe),
        )
        .route(
            "/webmention",
            post(site::webmention::receive).layer(DefaultBodyLimit::max(16 * 1024)),
        )
        .route("/rss.xml", get(site::feeds::rss))
        .route("/sitemap.xml", get(site::feeds::sitemap))
        .route("/robots.txt", get(site::feeds::robots_txt))
        .route("/favicon.ico", get(site::feeds::favicon))
        .route("/health", get(site::feeds::health))
        .route("/static/*path", get(assets::serve_static))
        .nest("/uploads", uploads)
        .nest("/theme-assets", theme_assets)
        // Standalone pages (About, Contact...). Listed last among public
        // routes; the fixed paths above take priority.
        .route("/:slug", get(site::pages::show_page))
}

/// The admin panel. Everything except login and setup needs a signed-in person.
fn admin_routes(state: &AppState) -> Result<Router<AppState>, String> {
    // Admins only.
    let admins = Router::new()
        .route("/admin/settings", get(admin::settings::page))
        .route(
            "/admin/settings/general",
            post(admin::settings::update_general),
        )
        .route("/admin/settings/theme", post(admin::settings::update_theme))
        .route("/admin/settings/email", post(admin::settings::update_email))
        .route(
            "/admin/settings/email/test",
            post(admin::settings::send_test_email),
        )
        .route("/admin/subscribers", get(admin::subscribers::page))
        .route(
            "/admin/subscribers.csv",
            get(admin::subscribers::export_csv),
        )
        .route("/admin/subscribers/:id", delete(admin::subscribers::delete))
        .route(
            "/admin/users",
            get(admin::users::page).post(admin::users::create),
        )
        .route("/admin/users/:id", delete(admin::users::delete))
        .route("/admin/users/:id/role", post(admin::users::update_role))
        .route(
            "/admin/users/:id/password",
            post(admin::users::reset_password),
        )
        .route_layer(from_fn(middleware::require_admin));

    // Admins and editors.
    let editors = Router::new()
        .route(
            "/admin/pages",
            get(admin::posts::list_pages).post(admin::posts::create_page),
        )
        .route("/admin/pages/new", get(admin::posts::new_page))
        .route(
            "/admin/pages/:id/edit",
            get(admin::posts::edit).post(admin::posts::update),
        )
        .route("/admin/pages/:id", delete(admin::posts::delete))
        .route("/admin/tags", get(admin::tags::page))
        .route("/admin/tags/new", post(admin::tags::create))
        .route("/admin/tags/:id", delete(admin::tags::delete))
        .route("/admin/media/:id", delete(admin::media::delete))
        .route("/admin/comments", get(admin::comments::page))
        .route("/admin/comments/:id", delete(admin::comments::delete))
        .route(
            "/admin/comments/:id/:action",
            post(admin::comments::set_status),
        )
        .route_layer(from_fn(middleware::require_editor));

    // Everyone signed in, authors included. Post handlers also check who
    // owns each post.
    let upload_limit = DefaultBodyLimit::max(admin::media::MAX_UPLOAD_BYTES);
    let writers = Router::new()
        .route("/admin", get(admin::dashboard::page))
        .route(
            "/admin/profile",
            get(admin::profile::page).post(admin::profile::update),
        )
        .route(
            "/admin/profile/password",
            post(admin::profile::update_password),
        )
        .route("/admin/profile/tokens", post(admin::profile::create_token))
        .route(
            "/admin/profile/tokens/:id",
            delete(admin::profile::revoke_token),
        )
        .route(
            "/admin/posts",
            get(admin::posts::list_posts).post(admin::posts::create_post),
        )
        .route("/admin/posts/new", get(admin::posts::new_post))
        .route(
            "/admin/posts/:id/edit",
            get(admin::posts::edit).post(admin::posts::update),
        )
        .route("/admin/posts/:id", delete(admin::posts::delete))
        .route(
            "/admin/posts/:id/revisions/:rev",
            get(admin::posts::revision),
        )
        .route("/admin/preview", post(admin::posts::preview))
        .route(
            "/admin/media",
            get(admin::media::page)
                .post(admin::media::upload)
                .layer(upload_limit),
        )
        .route(
            "/admin/media/picker",
            get(admin::media::picker)
                .post(admin::media::picker_upload)
                .layer(upload_limit),
        );

    let signed_in = Router::new()
        .merge(admins)
        .merge(editors)
        .merge(writers)
        .route_layer(from_fn_with_state(state.clone(), middleware::require_login));

    Ok(Router::new()
        .merge(signed_in)
        .route("/admin/login", login_route(state)?)
        .route("/admin/logout", get(admin::auth::logout))
        .route("/setup", get(admin::setup::page).post(admin::setup::submit))
        .layer(SetResponseHeaderLayer::overriding(
            header::CONTENT_SECURITY_POLICY,
            HeaderValue::from_static(ADMIN_CSP),
        )))
}

/// GET and POST /admin/login. Password guessing is slowed down: 5 quick
/// attempts per address, then one more every 12 seconds.
///
/// Behind a local reverse proxy the visitor's address comes from
/// `X-Forwarded-For`; when Bloogla is reached directly that header could be
/// forged, so the connection's own address is used instead.
fn login_route(state: &AppState) -> Result<MethodRouter<AppState>, String> {
    const ERROR: &str = "invalid login rate limit";
    let login = post(admin::auth::login);
    let login = if state.config.host.is_loopback() {
        let limit = GovernorConfigBuilder::default()
            .per_second(12)
            .burst_size(5)
            .key_extractor(SmartIpKeyExtractor)
            .finish()
            .ok_or(ERROR)?;
        login.layer(GovernorLayer {
            config: Arc::new(limit),
        })
    } else {
        let limit = GovernorConfigBuilder::default()
            .per_second(12)
            .burst_size(5)
            .finish()
            .ok_or(ERROR)?;
        login.layer(GovernorLayer {
            config: Arc::new(limit),
        })
    };
    // Added after the layer, so showing the form isn't rate-limited.
    Ok(login.get(admin::auth::login_page))
}

/// The JSON API. Reading is public (and open to other sites); writing needs a token.
fn api_routes(state: &AppState) -> Router<AppState> {
    let read = Router::new()
        .route("/api/posts", get(api::list_posts))
        // Same pattern as the write route below: a slug when reading, an id when writing.
        .route("/api/posts/:key", get(api::get_post))
        .route("/api/pages/:slug", get(api::get_page))
        .route("/api/tags", get(api::list_tags))
        .route("/api/*rest", any(api::not_found))
        .route_layer(SetResponseHeaderLayer::overriding(
            header::ACCESS_CONTROL_ALLOW_ORIGIN,
            HeaderValue::from_static("*"),
        ));
    let write = Router::new()
        .route("/api/me", get(api::me))
        .route("/api/posts", post(api::create_post))
        .route("/api/posts/:key", put(api::update).delete(api::delete))
        .route_layer(from_fn_with_state(
            state.clone(),
            middleware::require_api_token,
        ));
    read.merge(write)
}

/// Headers that tell browsers to lock the site down: no framing by other
/// sites, no content-type guessing, no camera/microphone/location access.
fn with_security_headers(app: Router<AppState>, https: bool) -> Router<AppState> {
    let app = app
        .layer(SetResponseHeaderLayer::overriding(
            header::X_FRAME_OPTIONS,
            HeaderValue::from_static("DENY"),
        ))
        .layer(SetResponseHeaderLayer::overriding(
            header::X_CONTENT_TYPE_OPTIONS,
            HeaderValue::from_static("nosniff"),
        ))
        .layer(SetResponseHeaderLayer::overriding(
            header::REFERRER_POLICY,
            HeaderValue::from_static("strict-origin-when-cross-origin"),
        ))
        .layer(SetResponseHeaderLayer::if_not_present(
            header::CONTENT_SECURITY_POLICY,
            HeaderValue::from_static(
                "frame-ancestors 'none'; base-uri 'self'; object-src 'none'; form-action 'self'",
            ),
        ))
        .layer(SetResponseHeaderLayer::overriding(
            HeaderName::from_static("permissions-policy"),
            HeaderValue::from_static(
                "camera=(), microphone=(), geolocation=(), interest-cohort=()",
            ),
        ));
    // Only tell browsers to insist on HTTPS when the site is actually served over it.
    if https {
        app.layer(SetResponseHeaderLayer::overriding(
            header::STRICT_TRANSPORT_SECURITY,
            HeaderValue::from_static("max-age=31536000"),
        ))
    } else {
        app
    }
}

#[cfg(test)]
mod tests {
    /// The admin CSP blocks inline scripts, so they must not creep back into
    /// the templates (they would silently stop working).
    #[test]
    fn admin_templates_have_no_inline_scripts() {
        let dir = concat!(env!("CARGO_MANIFEST_DIR"), "/admin/templates");
        for entry in std::fs::read_dir(dir).unwrap() {
            let path = entry.unwrap().path();
            let html = std::fs::read_to_string(&path).unwrap().to_lowercase();
            let name = path.display();
            assert!(
                !html.contains("<script>"),
                "{name}: inline <script>; use a file in admin/static/js"
            );
            assert!(!html.contains("hx-on"), "{name}: hx-on runs inline code");
            assert!(!html.contains("javascript:"), "{name}: javascript: URL");
            for handler in [
                " onclick=",
                " onchange=",
                " oninput=",
                " onsubmit=",
                " onload=",
                " onerror=",
            ] {
                assert!(!html.contains(handler), "{name}: inline{handler} handler");
            }
        }
    }
}
