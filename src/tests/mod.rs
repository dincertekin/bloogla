//! Tests that use Bloogla the way a browser does: they send real requests to
//! the whole app (routes, middleware, database, templates) and look at the
//! answers. Run them with `cargo test`.
//!
//! Each test makes its own [`TestSite`] with a fresh database, so tests don't
//! affect each other. To add a test, copy one from a file next to this one.
//!
//! - `badges`: numbers next to menu items, the new-version notice
//! - `setup_and_login`: first-run setup, signing in and out, password reset
//! - `posts`: writing posts and what visitors see
//! - `security`: CSRF, security headers, caching
//! - `settings`: saving settings and your profile
//! - `themes`: every bundled theme draws every page

mod badges;
mod posts;
mod security;
mod settings;
mod setup_and_login;
mod themes;

use crate::app::config::Config;
use crate::app::state::AppState;

use axum::body::Body;
use axum::extract::ConnectInfo;
use axum::http::{header, HeaderMap, Request, StatusCode};
use axum::Router;
use sqlx::SqlitePool;
use std::net::SocketAddr;
use std::path::PathBuf;
use std::sync::atomic::AtomicBool;
use std::sync::{Arc, RwLock};
use tower::ServiceExt;
use tower_sessions::SessionManagerLayer;
use tower_sessions_sqlx_store::SqliteStore;

pub const ADMIN_EMAIL: &str = "owner@example.com";
pub const ADMIN_PASSWORD: &str = "Correct-Horse-9-Battery";
pub const SETUP_CODE: &str = "test-setup-code";

/// Site settings are cached in memory for the whole program, so tests that
/// run a site take turns instead of running at the same time.
static ONE_AT_A_TIME: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

/// A complete Bloogla site with its own database, and one visitor's browser
/// (it keeps the sign-in cookie between requests).
pub struct TestSite {
    app: Router,
    pub pool: SqlitePool,
    /// What the daily update check found; tests set it instead of asking GitHub.
    pub newer_release: crate::services::updates::NewerRelease,
    /// The visitor's session cookie, once the site has set one.
    cookie: Option<String>,
    /// Each site's visitor has its own address, so rate limits don't mix.
    address: SocketAddr,
    database: PathBuf,
    _turn: tokio::sync::MutexGuard<'static, ()>,
}

/// What the site answered.
pub struct Page {
    pub status: StatusCode,
    pub headers: HeaderMap,
    pub body: String,
}

impl Page {
    /// Where a redirect points.
    pub fn location(&self) -> &str {
        self.headers
            .get(header::LOCATION)
            .and_then(|v| v.to_str().ok())
            .unwrap_or_default()
    }

    pub fn header(&self, name: &str) -> &str {
        self.headers
            .get(name)
            .and_then(|v| v.to_str().ok())
            .unwrap_or_default()
    }
}

impl TestSite {
    /// A brand-new site that hasn't been set up yet.
    pub async fn new() -> Self {
        let turn = ONE_AT_A_TIME.lock().await;
        crate::db::settings::invalidate();

        let database = std::env::temp_dir().join(format!(
            "bloogla-test-{}.db",
            crate::app::security::random_hex(8)
        ));
        let pool = crate::db::connect(&format!("sqlite://{}?mode=rwc", database.display()))
            .await
            .expect("test database");
        let state = AppState {
            pool: pool.clone(),
            config: Config::default(),
            themes: Arc::new(RwLock::new(crate::services::themes::Themes::load())),
            setup_pending: Arc::new(AtomicBool::new(true)),
            setup_code: SETUP_CODE.into(),
            newer_release: Default::default(),
        };
        let newer_release = state.newer_release.clone();
        let store = SqliteStore::new(pool.clone());
        store.migrate().await.expect("session table");
        let app =
            crate::server::routes::build(state, SessionManagerLayer::new(store)).expect("routes");

        let random = crate::app::security::random_hex(3);
        let bytes = u32::from_str_radix(&random, 16).unwrap_or(1).to_be_bytes();
        let address = SocketAddr::from(([10, bytes[1], bytes[2], bytes[3]], 50000));

        TestSite {
            app,
            pool,
            newer_release,
            cookie: None,
            address,
            database,
            _turn: turn,
        }
    }

    /// A site that's been set up, with the owner signed in.
    pub async fn with_owner() -> Self {
        let mut site = Self::new().await;
        let page = site.finish_setup(SETUP_CODE, ADMIN_PASSWORD).await;
        assert_eq!(page.location(), "/admin", "setup failed");
        site
    }

    /// Send the setup form.
    pub async fn finish_setup(&mut self, code: &str, password: &str) -> Page {
        self.post(
            "/setup",
            &[
                ("code", code),
                ("language", "en"),
                ("blog_name", "Test Blog"),
                ("name", "Owner"),
                ("email", ADMIN_EMAIL),
                ("password", password),
                ("confirm_password", password),
            ],
        )
        .await
    }

    /// Sign in with an email and password.
    pub async fn sign_in(&mut self, email: &str, password: &str) -> Page {
        self.post("/admin/login", &[("email", email), ("password", password)])
            .await
    }

    /// Forget the cookie, like a different visitor.
    pub fn new_visitor(&mut self) {
        self.cookie = None;
    }

    /// Publish a post from the editor and return its address (`/post/slug`).
    pub async fn publish(&mut self, title: &str, status: &str) -> String {
        let page = self
            .post(
                "/admin/posts",
                &[
                    ("title", title),
                    ("content", "Some **text** to read."),
                    ("status", status),
                ],
            )
            .await;
        assert_eq!(page.status, StatusCode::SEE_OTHER, "saving {title} failed");
        let slug = crate::content::text::slugify(title);
        format!("/post/{slug}")
    }

    pub async fn get(&mut self, path: &str) -> Page {
        self.send(Request::get(path), Body::empty()).await
    }

    pub async fn post(&mut self, path: &str, fields: &[(&str, &str)]) -> Page {
        let body: Vec<String> = fields
            .iter()
            .map(|(k, v)| {
                let encode = crate::content::text::url_encode;
                format!("{}={}", encode(k), encode(v))
            })
            .collect();
        let request = Request::post(path)
            .header(header::CONTENT_TYPE, "application/x-www-form-urlencoded")
            .header("sec-fetch-site", "same-origin");
        self.send(request, Body::from(body.join("&"))).await
    }

    /// Send any request (with this visitor's cookie) and read the answer.
    pub async fn send(&mut self, request: axum::http::request::Builder, body: Body) -> Page {
        let mut request = request;
        if let Some(cookie) = &self.cookie {
            request = request.header(header::COOKIE, cookie);
        }
        let mut request = request.body(body).expect("request");
        request.extensions_mut().insert(ConnectInfo(self.address));

        let response = self.app.clone().oneshot(request).await.expect("response");
        if let Some(cookie) = response
            .headers()
            .get(header::SET_COOKIE)
            .and_then(|v| v.to_str().ok())
        {
            // Keep `id=...`, drop the attributes after `;`.
            self.cookie = cookie.split(';').next().map(str::to_string);
        }
        let status = response.status();
        let headers = response.headers().clone();
        let bytes = axum::body::to_bytes(response.into_body(), usize::MAX)
            .await
            .expect("body");
        Page {
            status,
            headers,
            body: String::from_utf8_lossy(&bytes).into_owned(),
        }
    }
}

impl Drop for TestSite {
    fn drop(&mut self) {
        for suffix in ["", "-wal", "-shm"] {
            let _ = std::fs::remove_file(format!("{}{suffix}", self.database.display()));
        }
    }
}
