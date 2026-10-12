//! Protection against other sites, and headers that keep browsers safe and fast.

use super::TestSite;
use axum::body::Body;
use axum::http::{header, HeaderMap, HeaderValue, Request, StatusCode};

// The rate limits (login, comments, password reset) and the security log all
// key on the visitor's address. Behind a proxy that address is read from
// `X-Forwarded-For`, where the visitor writes the first entry and the proxy
// adds the real one last. Reading the first let a visitor change it on every
// request and slip past the limits, so the last entry is the one to trust.
#[test]
fn forwarded_for_reads_the_proxys_entry_not_the_visitors() {
    let mut headers = HeaderMap::new();
    headers.insert(
        "x-forwarded-for",
        HeaderValue::from_static("10.9.9.9, 203.0.113.7"),
    );
    assert_eq!(
        crate::handlers::forwarded_for(&headers).map(|ip| ip.to_string()),
        Some("203.0.113.7".to_string()),
        "a forged first entry must not hide the proxy's real one",
    );
}

#[test]
fn forwarded_for_reads_the_proxys_own_header_line() {
    // Some proxies add their address as a new header line instead.
    let mut headers = HeaderMap::new();
    headers.append("x-forwarded-for", HeaderValue::from_static("10.9.9.9"));
    headers.append("x-forwarded-for", HeaderValue::from_static("203.0.113.7"));
    assert_eq!(
        crate::handlers::forwarded_for(&headers).map(|ip| ip.to_string()),
        Some("203.0.113.7".to_string()),
    );
}

#[tokio::test]
async fn a_forged_forwarded_for_cannot_dodge_the_login_limit() {
    let mut site = TestSite::behind_proxy().await;
    site.finish_setup(super::SETUP_CODE, super::ADMIN_PASSWORD)
        .await;
    site.new_visitor();
    // One visitor (the proxy adds the same real address each time), guessing a
    // different account each time so only the per-address limit applies, and
    // writing a different made-up address first on every request.
    let mut statuses = Vec::new();
    for n in 1..=8 {
        let request = Request::post("/admin/login")
            .header(header::CONTENT_TYPE, "application/x-www-form-urlencoded")
            .header("sec-fetch-site", "same-origin")
            .header("x-forwarded-for", format!("10.0.0.{n}, 203.0.113.7"));
        let body = format!("email=user{n}%40example.com&password=wrong");
        statuses.push(site.send(request, Body::from(body)).await.status);
    }
    assert!(
        statuses.contains(&StatusCode::TOO_MANY_REQUESTS),
        "a made-up address got past the login limit: {statuses:?}"
    );
}

#[tokio::test]
async fn forms_from_other_sites_are_refused() {
    let mut site = TestSite::with_owner().await;
    let request = Request::post("/admin/posts")
        .header(header::CONTENT_TYPE, "application/x-www-form-urlencoded")
        .header("sec-fetch-site", "cross-site");
    let page = site.send(request, Body::from("title=Hacked")).await;
    assert_eq!(page.status, StatusCode::FORBIDDEN);
}

#[tokio::test]
async fn pages_have_security_headers() {
    let mut site = TestSite::with_owner().await;
    for path in ["/", "/admin"] {
        let page = site.get(path).await;
        assert_eq!(page.header("x-frame-options"), "DENY", "{path}");
        assert_eq!(page.header("x-content-type-options"), "nosniff", "{path}");
        assert!(
            page.header("content-security-policy")
                .contains("default-src 'self'"),
            "{path}"
        );
    }
}

#[tokio::test]
async fn repeat_visits_get_304_not_modified() {
    let mut site = TestSite::with_owner().await;
    site.new_visitor();
    let first = site.get("/").await;
    let etag = first.header("etag").to_string();
    assert!(!etag.is_empty());

    let again = site
        .send(
            Request::get("/").header(header::IF_NONE_MATCH, &etag),
            Body::empty(),
        )
        .await;
    assert_eq!(again.status, StatusCode::NOT_MODIFIED);
}

#[tokio::test]
async fn admins_can_download_a_backup() {
    let mut site = TestSite::with_owner().await;
    let page = site.get("/admin/backup.zip").await;
    assert_eq!(page.status, StatusCode::OK);
    assert_eq!(page.header("content-type"), "application/zip");
    assert!(page.body.contains("bloogla.db"), "the database is inside");

    site.new_visitor();
    assert_eq!(
        site.get("/admin/backup.zip").await.location(),
        "/admin/login"
    );
}
