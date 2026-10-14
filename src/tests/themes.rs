//! Every bundled theme draws every kind of page. Templates are only checked
//! for mistakes (like a misspelled variable) when they're used, so this
//! opens each page with real content.

use super::TestSite;
use axum::http::StatusCode;

#[tokio::test]
async fn every_bundled_theme_shows_every_page() {
    // Each theme, what its front page shows, and one of its font files.
    for (theme, front_page, font) in [
        (
            "default",
            &["featured", "card-grid", "topics"][..],
            "inter-latin-wght-normal.woff2",
        ),
        (
            "story",
            &["feed-item", "feed-thumb", "topics"][..],
            "source-serif-4-latin-wght-normal.woff2",
        ),
        (
            "gazette",
            &["masthead", r#"class="lead""#, "More stories"][..],
            "source-serif-4-latin-wght-normal.woff2",
        ),
        (
            "folio",
            &[r#"class="lead""#, r#"class="tiles""#][..],
            "archivo-latin-wght-normal.woff2",
        ),
        (
            "notebook",
            &[r#"class="year""#, r#"class="entries""#][..],
            "newsreader-latin-wght-normal.woff2",
        ),
        (
            "atelier",
            &[r#"class="cards""#, r#"class="card""#][..],
            "fraunces-latin-wght-normal.woff2",
        ),
        (
            "brief",
            &[r#"class="front""#, r#"class="briefs""#, r#"class="rows""#][..],
            "public-sans-latin-wght-normal.woff2",
        ),
    ] {
        let mut site = TestSite::with_owner().await;
        let activated = site
            .post("/admin/themes/activate", &[("theme", theme)])
            .await;
        assert_eq!(
            activated.status,
            StatusCode::SEE_OTHER,
            "{theme}: activate\n{}",
            activated.body
        );

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
        let font = site
            .get(&format!("/theme-assets/{theme}/static/fonts/{font}"))
            .await;
        assert_eq!(font.status, StatusCode::OK, "{theme}: font");
        assert_eq!(
            font.header("content-type"),
            "font/woff2",
            "{theme}: font type"
        );
        // A post links to the posts before and after it; a page doesn't.
        let post = site.get("/post/post-number-3").await.body;
        assert!(
            post.contains(r#"href="&#x2F;post&#x2F;post-number-2" rel="prev""#),
            "{theme}: previous"
        );
        assert!(
            post.contains(r#"href="&#x2F;post&#x2F;post-number-4" rel="next""#),
            "{theme}: next"
        );
        assert!(
            !site.get("/about").await.body.contains("post-nav"),
            "{theme}: page"
        );
        let missing = site.get("/no-such-page").await;
        assert_eq!(missing.status, StatusCode::NOT_FOUND, "{theme}: 404");
        assert!(missing.body.contains("<html"), "{theme}: themed 404");
    }
}

#[tokio::test]
async fn the_newest_and_oldest_posts_link_one_way() {
    let mut site = TestSite::with_owner().await;
    for title in ["First", "Second"] {
        site.post(
            "/admin/posts",
            &[
                ("title", title),
                ("content", "Text"),
                ("status", "published"),
            ],
        )
        .await;
    }
    let first = site.get("/post/first").await.body;
    assert!(first.contains(r#"href="&#x2F;post&#x2F;second" rel="next""#));
    assert!(!first.contains(r#"rel="prev""#));
    let second = site.get("/post/second").await.body;
    assert!(second.contains(r#"href="&#x2F;post&#x2F;first" rel="prev""#));
    assert!(!second.contains(r#"rel="next""#));
}

#[tokio::test]
async fn appearance_sets_the_logo_light_or_dark_and_a_readable_accent() {
    let mut site = TestSite::with_owner().await;
    let home = site.get("/").await.body;
    assert!(home.contains(r#"<meta name="color-scheme" content="light dark">"#));
    assert!(home.contains(r#"<html lang="en">"#));
    // The accent gets a lighter shade for dark mode.
    assert!(home.contains("color-mix(in srgb,#2563eb 55%,white)"));

    let saved = site
        .post(
            "/admin/themes/default/options",
            &[
                ("logo", "/uploads/logo.png"),
                ("color_scheme", "Always dark"),
                ("accent_color", "#1e3a8a"),
                ("footer_text", ""),
                ("show_powered_by", "true"),
            ],
        )
        .await;
    assert!(!saved.body.contains("error-message"), "{}", saved.body);

    site.new_visitor();
    let home = site.get("/").await.body;
    assert!(home.contains(r#"<html lang="en" data-scheme="dark">"#));
    assert!(home.contains(r#"<meta name="color-scheme" content="dark">"#));
    assert!(home.contains(r#"<img class="site-logo" src="&#x2F;uploads&#x2F;logo.png""#));
    assert!(home.contains("color-mix(in srgb,#1e3a8a 55%,white)"));
}

#[tokio::test]
async fn appearance_refuses_a_light_or_dark_choice_not_on_the_list() {
    let mut site = TestSite::with_owner().await;
    let saved = site
        .post(
            "/admin/themes/default/options",
            &[
                ("logo", ""),
                ("color_scheme", "Purple"),
                ("accent_color", "#2563eb"),
                ("footer_text", ""),
            ],
        )
        .await;
    assert!(saved.body.contains("Pick one of the choices."));
    assert!(site.get("/").await.body.contains(r#"<html lang="en">"#));
}
