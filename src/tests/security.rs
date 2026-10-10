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
        assert_eq!(
            page.header("referrer-policy"),
            "strict-origin-when-cross-origin",
            "{path}"
        );
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
    assert!(
        page.body.contains("data/bloogla.db"),
        "the database uses the startup path"
    );

    site.new_visitor();
    assert_eq!(
        site.get("/admin/backup.zip").await.location(),
        "/admin/login"
    );
}

#[tokio::test]
async fn an_extracted_backup_restores_the_site_database() {
    let mut site = TestSite::with_owner().await;
    site.publish("A Post To Restore", "published").await;
    let folder = std::env::temp_dir().join(format!(
        "bloogla-restore-test-{}",
        crate::app::security::random_hex(8)
    ));
    // Keep the test's uploads separate from any real site's pictures.
    let uploads = folder.join("source-uploads");
    std::fs::create_dir_all(&uploads).unwrap();
    std::fs::write(uploads.join("picture.png"), b"test picture").unwrap();
    let archive = crate::services::backup::download_archive(&site.pool, &uploads)
        .await
        .unwrap();
    let restored = folder.join("restored");
    let mut zip = zip::ZipArchive::new(std::fs::File::open(&archive).unwrap()).unwrap();
    zip.extract(&restored).unwrap();
    drop(zip);
    std::fs::remove_file(archive).unwrap();

    let relative_database = crate::app::config::Config::default().database_url;
    let relative_database = relative_database
        .strip_prefix("sqlite://")
        .unwrap()
        .split('?')
        .next()
        .unwrap();
    let database = restored.join(relative_database);
    assert!(
        database.is_file(),
        "startup must find the restored database"
    );
    let pool = crate::db::connect(&format!("sqlite://{}?mode=rwc", database.display()))
        .await
        .unwrap();
    let post = crate::db::posts::find_public(&pool, "a-post-to-restore", false)
        .await
        .expect("the published post survives restoration");
    assert_eq!(post.title, "A Post To Restore");
    assert_eq!(
        std::fs::read(restored.join("uploads/picture.png")).unwrap(),
        b"test picture"
    );
    pool.close().await;
    std::fs::remove_dir_all(folder).unwrap();
}

#[tokio::test]
async fn invalid_reset_pages_also_keep_tokens_out_of_referrers() {
    let mut site = TestSite::with_owner().await;
    let page = site.get("/admin/reset-password?token=invalid").await;
    assert_eq!(page.header("referrer-policy"), "no-referrer");
    assert_eq!(page.header("cache-control"), "no-store");
}
