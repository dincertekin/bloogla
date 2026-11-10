//! The demo content `cargo run` adds after setup: it saves, and every
//! bundled theme draws it. (Setup in tests doesn't add it.)

use super::TestSite;
use axum::http::StatusCode;

#[tokio::test]
async fn the_demo_content_fills_a_site_that_every_theme_draws() {
    let mut site = TestSite::with_owner().await;
    crate::services::demo::add(&site.pool, 1).await;

    let count = |sql: &'static str| {
        let pool = site.pool.clone();
        async move {
            sqlx::query_scalar::<_, i64>(sql)
                .fetch_one(&pool)
                .await
                .unwrap()
        }
    };
    assert_eq!(
        count("SELECT COUNT(*) FROM posts WHERE is_page = 1").await,
        4
    );
    assert_eq!(
        count("SELECT COUNT(*) FROM posts WHERE is_page = 0").await,
        11
    );
    assert_eq!(
        count("SELECT COUNT(*) FROM comments WHERE status = 'pending'").await,
        1
    );
    assert_eq!(count("SELECT COUNT(*) FROM messages").await, 1);

    let themes = [
        "atelier", "brief", "default", "folio", "gazette", "notebook", "story",
    ];
    for theme in themes {
        let activated = site
            .post("/admin/themes/activate", &[("theme", theme)])
            .await;
        assert_eq!(activated.status, StatusCode::SEE_OTHER, "{theme}: activate");
        for path in [
            "/",
            "/about",
            "/contact",
            "/archive",
            "/post/a-blog-that-doesnt-watch",
            "/tag/rust",
        ] {
            let page = site.get(path).await;
            assert!(!page.body.contains("Template error"), "{theme}: {path}");
            assert_eq!(page.status, StatusCode::OK, "{theme}: {path}");
        }
    }
    let home = site.get("/").await.body;
    assert!(home.contains("Notes on building Bloogla"));
    // Drafts and scheduled posts stay hidden.
    assert!(!home.contains("Ideas for Bloogla 1.5"));
    assert_eq!(
        site.get("/post/signing-s3-requests-by-hand").await.status,
        StatusCode::NOT_FOUND
    );
    // The contact page has its form.
    assert!(site.get("/contact").await.body.contains("contact-form"));
}
