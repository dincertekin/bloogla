//! Settings → Redirects: old addresses that send visitors to new ones.

use super::{TestSite, ADMIN_EMAIL, ADMIN_PASSWORD};
use axum::body::Body;
use axum::http::{Request, StatusCode};

#[tokio::test]
async fn an_old_address_leads_to_the_new_one() {
    let mut site = TestSite::with_owner().await;
    let address = site.publish("Opening hours", "published").await;

    let added = site
        .post(
            "/admin/redirects",
            &[
                ("from", "https://old-site.example/hours.html?ref=menu"),
                ("to", &address),
            ],
        )
        .await;
    assert_eq!(added.status, StatusCode::OK);
    assert!(added.body.contains("/hours.html"));
    assert!(site
        .get("/admin/settings")
        .await
        .body
        .contains("/hours.html"));

    // Letters like ı work in both addresses, and so do links to other sites.
    site.post(
        "/admin/redirects",
        &[("from", "/hakkımızda/"), ("to", "https://example.com/biz")],
    )
    .await;

    site.new_visitor();
    let old = site.get("/hours.html").await;
    assert_eq!(old.status, StatusCode::MOVED_PERMANENTLY);
    assert_eq!(old.location(), address);
    let turkish = site.get("/hakk%C4%B1m%C4%B1zda").await;
    assert_eq!(turkish.status, StatusCode::MOVED_PERMANENTLY);
    assert_eq!(turkish.location(), "https://example.com/biz");
    assert_eq!(site.get("/not-listed").await.status, StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn a_redirect_never_hides_a_real_page() {
    let mut site = TestSite::with_owner().await;
    let address = site.publish("Still here", "published").await;
    site.post(
        "/admin/redirects",
        &[("from", &address), ("to", "/elsewhere")],
    )
    .await;
    site.new_visitor();
    assert_eq!(site.get(&address).await.status, StatusCode::OK);
}

#[tokio::test]
async fn unusable_redirects_are_refused_in_plain_words() {
    let mut site = TestSite::with_owner().await;
    for (from, to, message) in [
        ("old-page", "/new", "like /old-page"),
        ("/admin/posts", "/new", "like /old-page"),
        (
            "/old",
            "javascript:alert(1)",
            "like /about or https://example.com/page",
        ),
        ("/old", "/old/", "must be different"),
    ] {
        let refused = site
            .post("/admin/redirects", &[("from", from), ("to", to)])
            .await;
        assert!(refused.body.contains(message), "{from} → {to}");
        assert!(refused.body.contains("alert-error"));
    }
    site.post("/admin/redirects", &[("from", "/old"), ("to", "/new")])
        .await;
    let twice = site
        .post("/admin/redirects", &[("from", "/old/"), ("to", "/other")])
        .await;
    assert!(twice.body.contains("already leads somewhere"));
}

#[tokio::test]
async fn a_deleted_redirect_stops_working_and_only_admins_change_them() {
    let mut site = TestSite::with_owner().await;
    site.post("/admin/redirects", &[("from", "/old"), ("to", "/new")])
        .await;
    let id: i64 = sqlx::query_scalar("SELECT id FROM redirects")
        .fetch_one(&site.pool)
        .await
        .unwrap();

    let password = "Editor-Pass-123!";
    let hash = crate::app::security::hash_password(password).unwrap();
    sqlx::query("INSERT INTO users (email, password_hash, name, role) VALUES ('editor@example.com', ?, 'Editor', 'editor')")
        .bind(hash)
        .execute(&site.pool)
        .await
        .unwrap();
    site.new_visitor();
    site.sign_in("editor@example.com", password).await;
    let refused = site
        .post("/admin/redirects", &[("from", "/a"), ("to", "/b")])
        .await;
    assert_eq!(refused.status, StatusCode::FORBIDDEN);

    site.new_visitor();
    site.sign_in(ADMIN_EMAIL, ADMIN_PASSWORD).await;
    let delete =
        Request::delete(format!("/admin/redirects/{id}")).header("sec-fetch-site", "same-origin");
    assert_eq!(
        site.send(delete, Body::empty()).await.status,
        StatusCode::OK
    );
    site.new_visitor();
    assert_eq!(site.get("/old").await.status, StatusCode::NOT_FOUND);
}
