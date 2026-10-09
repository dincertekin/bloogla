//! Protection against other sites, and headers that keep browsers safe and fast.

use super::TestSite;
use axum::body::Body;
use axum::http::{header, Request, StatusCode};

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
