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
            "source-serif-4-latin-opsz-normal.woff2",
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
            "/archive",
        ] {
            let page = site.get(path).await;
            assert_eq!(
                page.status,
                StatusCode::OK,
                "{theme}: {path}\n{}",
                page.body
            );
            if path == "/archive" {
                assert!(page.body.contains("Post number 3"), "{theme}: archive");
            }
            if path == "/" {
                // Visitors can switch light and dark (on by default).
                assert!(page.body.contains("data-scheme-toggle"), "{theme}: button");
                assert!(page.body.contains("static/js/scheme.js"), "{theme}: script");
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
        // Kept for a day, so moving between pages doesn't ask for it again.
        assert_eq!(
            font.header("cache-control"),
            "public, max-age=86400",
            "{theme}: font cache"
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

#[test]
fn cover_pictures_keep_their_shape() {
    // Cover <img> tags carry the photo's own width and height, so the browser
    // can keep its space while it loads. A rule that gives a picture a shape
    // (aspect-ratio) must also say `height: auto`, or the height stays at the
    // photo's pixels (1080 for a 1920×1080 photo) and it becomes a tall strip.
    let themes = concat!(env!("CARGO_MANIFEST_DIR"), "/themes");
    for theme in std::fs::read_dir(themes).unwrap().flatten() {
        let css = theme.path().join("static/css/style.css");
        let Ok(source) = std::fs::read_to_string(&css) else {
            continue;
        };
        for rule in source.split('}') {
            let Some((selector, body)) = rule.split_once('{') else {
                continue;
            };
            let selector = selector.rsplit("*/").next().unwrap_or(selector);
            if selector.contains("img") && body.contains("aspect-ratio") {
                assert!(
                    body.contains("height"),
                    "{}: `{}` sets aspect-ratio without height: auto",
                    css.display(),
                    selector.trim()
                );
            }
        }
    }
}

#[tokio::test]
async fn the_light_dark_button_shows_only_when_visitors_can_choose() {
    let mut site = TestSite::with_owner().await;
    let save = |scheme: &'static str, button: bool| {
        let mut fields = vec![
            ("logo", ""),
            ("color_scheme", scheme),
            ("accent_color", "#2563eb"),
            ("footer_text", ""),
        ];
        if button {
            fields.push(("scheme_button", "true"));
        }
        fields
    };

    // The owner chose dark for everyone: nothing to switch.
    let fields = save("Always dark", true);
    site.post("/admin/themes/default/options", &fields).await;
    let home = site.get("/").await.body;
    assert!(!home.contains("data-scheme-toggle"));
    assert!(!home.contains("scheme.js"));

    // Following the device, with the button turned off in Appearance.
    let fields = save("Follow the visitor's device", false);
    site.post("/admin/themes/default/options", &fields).await;
    assert!(!site.get("/").await.body.contains("data-scheme-toggle"));

    // And turned on again.
    let fields = save("Follow the visitor's device", true);
    site.post("/admin/themes/default/options", &fields).await;
    let home = site.get("/").await.body;
    assert!(home.contains("data-scheme-toggle"));
    // It starts by following the device, the first of its three choices.
    assert!(home.contains(r#"aria-label="Light or dark: same as your device""#));
    assert!(home.contains(r#"data-dark="Light or dark: always dark""#));
}

/// The parts every bundled theme shares (`themes/_shared/`) are kept once:
/// pages still draw them and their files are still served at each theme's
/// own address, but `_shared` isn't a theme anyone can open.
#[tokio::test]
async fn shared_theme_parts_are_kept_once_and_used_by_every_theme() {
    let mut site = TestSite::with_owner().await;
    let url = site.publish("Talk About It", "published").await;
    for theme in ["default", "story"] {
        site.post("/admin/themes/activate", &[("theme", theme)])
            .await;
        // `templates/comments.html` and `pagination.html` come from `_shared`.
        assert!(
            site.get(&url).await.body.contains(r#"class="comments""#),
            "{theme}"
        );
        let script = site
            .get(&format!("/theme-assets/{theme}/static/js/prism.js"))
            .await;
        assert_eq!(script.status, StatusCode::OK, "{theme}");
        assert!(script.header("content-type").contains("javascript"));
        assert!(script.header("cache-control").contains("max-age"));
    }
    for path in [
        "/theme-assets/_shared/static/js/prism.js",
        "/theme-assets/story/static/js/not-a-file.js",
        "/theme-assets/story/static/../theme.toml",
    ] {
        assert_eq!(site.get(path).await.status, StatusCode::NOT_FOUND, "{path}");
    }
    assert!(!site.get("/admin/themes").await.body.contains("_shared"));
}

/// Every bundled theme prints only the writing and balances long titles.
#[test]
fn every_bundled_theme_prints_cleanly_and_balances_titles() {
    for theme in [
        "atelier", "brief", "default", "folio", "gazette", "notebook", "story",
    ] {
        let css = std::fs::read_to_string(format!(
            "{}/themes/{theme}/static/css/style.css",
            env!("CARGO_MANIFEST_DIR")
        ))
        .unwrap();
        assert!(css.contains("@media print"), "{theme}: print styles");
        assert!(
            css.contains("text-wrap: balance"),
            "{theme}: balanced titles"
        );
    }
}

/// Each theme's own options change what they say (Appearance → Customize).
#[tokio::test]
async fn each_themes_own_options_change_its_pages() {
    let mut site = TestSite::with_owner().await;
    // Enough for Gazette's grid below its lead story and the three beside it.
    for title in [
        "First Story",
        "Second Story",
        "Third Story",
        "Fourth Story",
        "Fifth Story",
        "Sixth Story",
    ] {
        site.post(
            "/admin/posts",
            &[
                ("title", title),
                ("content", "Text"),
                ("status", "published"),
                ("new_tags", "News"),
            ],
        )
        .await;
    }
    let pool = site.pool.clone();
    let set = |key: &'static str, value: &'static str| {
        let pool = pool.clone();
        async move {
            crate::db::settings::save(&pool, &[(key, value.to_string())])
                .await
                .unwrap()
        }
    };

    // (theme, page, what it shows at first, the option, its other value, what it shows then)
    let cases = [
        (
            "default",
            "/",
            r#"class="featured""#,
            "theme.default.show_featured",
            "false",
            "card-grid",
        ),
        (
            "story",
            "/",
            r#"class="sidebar""#,
            "theme.story.show_sidebar",
            "false",
            "no-sidebar",
        ),
        (
            "story",
            "/post/first-story",
            "<body>",
            "theme.story.text_size",
            "Large",
            r#"class="text-large""#,
        ),
        (
            "gazette",
            "/",
            r#"class="grid""#,
            "theme.gazette.front_columns",
            "Three",
            "grid-three",
        ),
        (
            "folio",
            "/",
            r#"class="tiles""#,
            "theme.folio.tile_shape",
            "Square",
            "tiles-square",
        ),
        (
            "notebook",
            "/",
            r#"class="year""#,
            "theme.notebook.group_by_year",
            "false",
            r#"class="entries""#,
        ),
    ];
    for (theme, page, before, key, value, after) in cases {
        site.post("/admin/themes/activate", &[("theme", theme)])
            .await;
        let shown = site.get(page).await;
        assert_eq!(shown.status, StatusCode::OK, "{theme} {page}");
        assert!(shown.body.contains(before), "{theme}: {before}");
        set(key, value).await;
        let changed = site.get(page).await.body;
        assert!(changed.contains(after), "{theme}: {key} = {value}");
    }
    // Notebook without years shows no year headings.
    assert!(!site.get("/").await.body.contains(r#"class="year""#));
    // Story without its column.
    site.post("/admin/themes/activate", &[("theme", "story")])
        .await;
    assert!(!site.get("/").await.body.contains(r#"class="sidebar""#));
}

/// Folio's picture-less tiles show apostrophes as apostrophes, not `&apos;`.
#[tokio::test]
async fn folio_tiles_show_text_escaped_once() {
    let mut site = TestSite::with_owner().await;
    site.post(
        "/admin/posts",
        &[
            ("title", "Quotes"),
            ("content", "It's <b>bold</b> & “quoted”."),
            ("status", "published"),
        ],
    )
    .await;
    // Older, so it's a tile and not the large lead story.
    sqlx::query("UPDATE posts SET published_at = datetime('now', '-1 day') WHERE slug = 'quotes'")
        .execute(&site.pool)
        .await
        .unwrap();
    site.publish("Newest", "published").await;
    site.post("/admin/themes/activate", &[("theme", "folio")])
        .await;
    let home = site.get("/").await.body;
    assert!(
        !home.contains("&amp;apos;") && !home.contains("&amp;#x27;"),
        "escaped twice"
    );
    assert!(!home.contains("<b>bold</b>"), "tags are stripped");
}
