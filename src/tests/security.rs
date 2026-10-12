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

#[tokio::test]
async fn a_downloaded_backup_restores_the_site() {
    use std::io::Read;

    let mut site = TestSite::with_owner().await;
    site.publish("Kept in the backup", "published").await;
    let page = site.get("/admin/backup.zip").await;
    assert_eq!(page.status, StatusCode::OK);

    // Unpacked into Bloogla's folder, the database has to land where Bloogla
    // reads it: data/bloogla.db.
    let mut zip = zip::ZipArchive::new(std::io::Cursor::new(page.bytes)).expect("a .zip");
    let mut database = Vec::new();
    zip.by_name("data/bloogla.db")
        .expect("the database is in data/")
        .read_to_end(&mut database)
        .expect("readable");

    let restored = std::env::temp_dir().join(format!(
        "bloogla-restore-{}.db",
        crate::app::security::random_hex(8)
    ));
    std::fs::write(&restored, database).expect("write");
    let options = sqlx::sqlite::SqliteConnectOptions::new()
        .filename(&restored)
        .read_only(true);
    let pool = sqlx::SqlitePool::connect_with(options).await.expect("open");
    let titles: Vec<String> = sqlx::query_scalar("SELECT title FROM posts")
        .fetch_all(&pool)
        .await
        .expect("posts");
    pool.close().await;
    let _ = std::fs::remove_file(&restored);
    assert!(titles.contains(&"Kept in the backup".to_string()));
}
