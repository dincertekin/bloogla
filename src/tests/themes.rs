//! Every bundled theme draws every kind of page. Templates are only checked
//! for mistakes (like a misspelled variable) when they're used, so this
//! opens each page with real content.

use super::TestSite;
use axum::http::StatusCode;

#[tokio::test]
async fn every_bundled_theme_shows_every_page() {
    for (theme, front_page) in [
        ("default", &["post-card"][..]),
        ("story", &["feed-item", "feed-thumb", "topics"][..]),
        (
            "gazette",
            &["masthead", r#"class="lead""#, "More stories"][..],
        ),
    ] {
        let mut site = TestSite::with_owner().await;
        let activated = site
            .post("/admin/themes/activate", &[("theme", theme)])
            .await;
        assert_eq!(activated.status, StatusCode::SEE_OTHER, "{theme}: activate");

        // Enough posts for a second page (10 per page), with tags, code and
        // a cover picture; and a standalone page.
        for n in 1..=12 {
            let title = format!("Post number {n}");
            let saved = site
                .post(
                    "/admin/posts",
                    &[
                        ("title", title.as_str()),
                        (
                            "content",
                            "Intro.\n\n```rust\nfn main() {}\n```\n\n> A quote.",
                        ),
                        ("status", "published"),
                        ("new_tags", "Travel, Food"),
                        ("cover_image", "https://example.com/cover.jpg"),
                    ],
                )
                .await;
            assert_eq!(saved.status, StatusCode::SEE_OTHER, "{theme}: {title}");
        }
        site.post(
            "/admin/pages",
            &[
                ("title", "About"),
                ("content", "Hello."),
                ("status", "published"),
            ],
        )
        .await;
        site.new_visitor();

        for path in [
            "/",
            "/?page=2",
            "/post/post-number-3",
            "/tag/travel",
            "/search?q=post",
            "/about",
        ] {
            let page = site.get(path).await;
            assert_eq!(
                page.status,
                StatusCode::OK,
                "{theme}: {path}\n{}",
                page.body
            );
            if path == "/" {
                for part in front_page {
                    assert!(page.body.contains(part), "{theme}: front page has {part}");
                }
            }
        }
        if theme == "gazette" {
            let older = site.get("/?page=2").await.body;
            assert!(
                older.contains("river-item"),
                "gazette: older posts as a list"
            );
        }
        if theme != "default" {
            let font = site
                .get(&format!(
                    "/theme-assets/{theme}/static/fonts/source-serif-4-latin-wght-normal.woff2"
                ))
                .await;
            assert_eq!(font.status, StatusCode::OK, "{theme}: font");
            assert_eq!(
                font.header("content-type"),
                "font/woff2",
                "{theme}: font type"
            );
        }
        let missing = site.get("/no-such-page").await;
        assert_eq!(missing.status, StatusCode::NOT_FOUND, "{theme}: 404");
        assert!(missing.body.contains("<html"), "{theme}: themed 404");
    }
}
