//! Writing posts, and what visitors (and search engines) see.

use super::TestSite;
use axum::http::StatusCode;

#[tokio::test]
async fn a_published_post_shows_up_everywhere() {
    let mut site = TestSite::with_owner().await;
    let url = site.publish("Hello World", "published").await;
    site.new_visitor();

    let post = site.get(&url).await;
    assert_eq!(post.status, StatusCode::OK);
    assert!(
        post.body.contains("<strong>text</strong>"),
        "Markdown is rendered"
    );
    assert!(
        post.body.contains("rel=\"canonical\""),
        "SEO tags are there"
    );

    for (path, what) in [
        ("/", "home page"),
        ("/rss.xml", "feed"),
        ("/sitemap.xml", "sitemap"),
        ("/search?q=hello", "search"),
    ] {
        let page = site.get(path).await;
        assert_eq!(page.status, StatusCode::OK, "{what}");
        assert!(
            page.body.contains("hello-world"),
            "the post is in the {what}"
        );
    }
}

#[tokio::test]
async fn drafts_stay_hidden() {
    let mut site = TestSite::with_owner().await;
    let url = site.publish("Secret Plans", "draft").await;
    site.new_visitor();

    assert_eq!(site.get(&url).await.status, StatusCode::NOT_FOUND);
    for path in ["/", "/rss.xml", "/sitemap.xml", "/search?q=secret"] {
        assert!(
            !site.get(path).await.body.contains("secret-plans"),
            "{path}"
        );
    }
}

#[tokio::test]
async fn a_changed_address_redirects_to_the_new_one() {
    let mut site = TestSite::with_owner().await;
    site.publish("First Title", "published").await;
    let id: i64 = sqlx::query_scalar("SELECT id FROM posts")
        .fetch_one(&site.pool)
        .await
        .unwrap();
    site.post(
        &format!("/admin/posts/{id}/edit"),
        &[
            ("title", "Better Title"),
            ("slug", "better-title"),
            ("content", "Text"),
            ("status", "published"),
        ],
    )
    .await;

    let old = site.get("/post/first-title").await;
    assert_eq!(old.status, StatusCode::PERMANENT_REDIRECT);
    assert_eq!(old.location(), "/post/better-title");
}

#[tokio::test]
async fn comments_wait_for_approval() {
    let mut site = TestSite::with_owner().await;
    let url = site.publish("Talk To Me", "published").await;
    site.new_visitor();

    // The form notes when the page was opened; people take a few seconds.
    let opened = (chrono::Utc::now().timestamp() - 30) * 1000;
    let sent = site
        .post(
            &format!("{url}/comments"),
            &[
                ("name", "Reader"),
                ("content", "Nice post, thanks!"),
                ("ts", &opened.to_string()),
            ],
        )
        .await;
    assert!(sent.location().contains("comment=pending"));
    assert!(!site.get(&url).await.body.contains("Nice post, thanks!"));

    let waiting: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM comments WHERE status = 'pending'")
        .fetch_one(&site.pool)
        .await
        .unwrap();
    assert_eq!(waiting, 1);
}

#[tokio::test]
async fn unknown_addresses_get_a_404_page() {
    let mut site = TestSite::with_owner().await;
    let page = site.get("/no/such/page").await;
    assert_eq!(page.status, StatusCode::NOT_FOUND);
    assert!(
        page.body.contains("<html"),
        "the theme's page, not a bare error"
    );
}

#[tokio::test]
async fn tags_typed_in_the_editor_are_created_once() {
    let mut site = TestSite::with_owner().await;
    for title in ["Ferry Day", "Ferry Night"] {
        let page = site
            .post(
                "/admin/posts",
                &[
                    ("title", title),
                    ("content", "Text"),
                    ("status", "published"),
                    // "travel" is the same tag as "Travel".
                    ("new_tags", "Travel, Istanbul"),
                    ("new_tags", "travel"),
                ],
            )
            .await;
        assert_eq!(page.status, StatusCode::SEE_OTHER);
    }
    let tags: Vec<String> = sqlx::query_scalar("SELECT name FROM tags ORDER BY name")
        .fetch_all(&site.pool)
        .await
        .unwrap();
    assert_eq!(tags, ["Istanbul", "Travel"]);
    site.new_visitor();
    let tag_page = site.get("/tag/travel").await;
    assert!(tag_page.body.contains("ferry-day") && tag_page.body.contains("ferry-night"));
}

#[tokio::test]
async fn tags_used_only_by_drafts_stay_private() {
    let mut site = TestSite::with_owner().await;
    for (title, status, tag) in [
        ("Hello world", "published", "Travel"),
        ("Announcing Project Falcon", "draft", "Project Falcon"),
    ] {
        let page = site
            .post(
                "/admin/posts",
                &[
                    ("title", title),
                    ("content", "Text"),
                    ("status", status),
                    ("new_tags", tag),
                ],
            )
            .await;
        assert_eq!(page.status, StatusCode::SEE_OTHER);
    }

    // Settings only suggests tags the site shows for the menu.
    let settings = site.get("/admin/settings").await;
    assert!(settings.body.contains("/tag/travel"));
    assert!(!settings.body.contains("/tag/project-falcon"));

    site.new_visitor();
    let home = site.get("/").await;
    assert!(home.body.contains("/tag/travel"));
    assert!(
        !home.body.contains("Project Falcon"),
        "a draft's tag is shown on the site"
    );
    assert_eq!(site.get("/tag/travel").await.status, StatusCode::OK);
    assert_eq!(
        site.get("/tag/project-falcon").await.status,
        StatusCode::NOT_FOUND
    );
}

#[tokio::test]
async fn publish_times_are_typed_in_the_site_time_zone() {
    let mut site = TestSite::with_owner().await;
    let saved = site
        .post(
            "/admin/settings/general",
            &[("blog_name", "Test Blog"), ("timezone", "Europe/Istanbul")],
        )
        .await;
    assert_eq!(saved.status, StatusCode::OK);

    // Scheduled for 09:00 in Istanbul (UTC+3).
    site.post(
        "/admin/posts",
        &[
            ("title", "Morning Post"),
            ("content", "Text"),
            ("status", "scheduled"),
            ("published_at", "2030-06-01T09:00"),
        ],
    )
    .await;
    let stored: String = sqlx::query_scalar("SELECT published_at FROM posts")
        .fetch_one(&site.pool)
        .await
        .unwrap();
    assert_eq!(stored, "2030-06-01 06:00:00");

    let id: i64 = sqlx::query_scalar("SELECT id FROM posts")
        .fetch_one(&site.pool)
        .await
        .unwrap();
    let editor = site.get(&format!("/admin/posts/{id}/edit")).await;
    assert!(
        editor.body.contains("value=\"2030-06-01T09:00\""),
        "shown in local time"
    );
}

#[tokio::test]
async fn views_are_counted_and_saved_together() {
    use crate::handlers::site::analytics::{record_view, save_views};
    let mut site = TestSite::with_owner().await;
    site.publish("Popular Post", "published").await;
    let id: i64 = sqlx::query_scalar("SELECT id FROM posts WHERE slug = 'popular-post'")
        .fetch_one(&site.pool)
        .await
        .unwrap();

    // Many visitors at once only change memory; one save writes them all.
    for _ in 0..250 {
        record_view(id, Some("news.example".to_string()));
    }
    save_views(&site.pool).await;

    let views: i64 = sqlx::query_scalar("SELECT views FROM posts WHERE id = ?")
        .bind(id)
        .fetch_one(&site.pool)
        .await
        .unwrap();
    let today: i64 = sqlx::query_scalar("SELECT views FROM daily_views WHERE post_id = ?")
        .bind(id)
        .fetch_one(&site.pool)
        .await
        .unwrap();
    let referred: i64 =
        sqlx::query_scalar("SELECT views FROM daily_referrers WHERE host = 'news.example'")
            .fetch_one(&site.pool)
            .await
            .unwrap();
    assert_eq!((views, today, referred), (250, 250, 250));
}

/// A tag page, or a later page of the home page, names itself as its real
/// address; pointing search engines at the home page would hide it from them.
#[tokio::test]
async fn lists_of_posts_have_their_own_canonical_address() {
    let mut site = TestSite::with_owner().await;
    site.post(
        "/admin/settings/general",
        &[("posts_per_page", "1"), ("nav_menu", "")],
    )
    .await;
    for title in ["First", "Second"] {
        site.post(
            "/admin/posts",
            &[
                ("title", title),
                ("content", "Text."),
                ("status", "published"),
                ("new_tags", "Recipes"),
            ],
        )
        .await;
    }
    site.new_visitor();
    let canonical = |body: &str| {
        let start = body
            .find(r#"<link rel="canonical" href=""#)
            .expect("a canonical link")
            + 28;
        body[start..start + body[start..].find('"').unwrap()].to_string()
    };
    let home = canonical(&site.get("/").await.body);
    assert!(!home.contains("page="), "{home}");
    let page_two = canonical(&site.get("/?page=2").await.body);
    assert!(page_two.ends_with("/?page=2"), "{page_two}");
    let tag = canonical(&site.get("/tag/recipes").await.body);
    assert!(tag.ends_with("/tag/recipes"), "{tag}");
    let tag_two = canonical(&site.get("/tag/recipes?page=2").await.body);
    assert!(tag_two.ends_with("/tag/recipes?page=2"), "{tag_two}");
}

/// The editor's writing area has a name for screen readers ("Post text" or
/// "Page text"; editor.js puts it on the writing area).
#[tokio::test]
async fn the_writing_area_is_named_for_screen_readers() {
    let mut site = TestSite::with_owner().await;
    let post = site.get("/admin/posts/new").await.body;
    assert!(post.contains(r#"data-text-writing-area="Post text""#));
    let page = site.get("/admin/pages/new").await.body;
    assert!(page.contains(r#"data-text-writing-area="Page text""#));
}
