//! A page as the home page (Settings → Reading): the page at `/`, the posts
//! at `/blog`.

use super::TestSite;
use axum::http::StatusCode;

/// The id of the newest post or page.
async fn newest_id(site: &TestSite) -> i64 {
    sqlx::query_scalar("SELECT MAX(id) FROM posts")
        .fetch_one(&site.pool)
        .await
        .unwrap()
}

/// Settings → Reading, saved with `home_page` (a page id, or empty).
async fn choose_home_page(site: &mut TestSite, home_page: &str) -> super::Page {
    site.post(
        "/admin/settings/general",
        &[
            ("home_page", home_page),
            ("posts_per_page", "1"),
            ("nav_menu", ""),
        ],
    )
    .await
}

/// A site with two posts and a published "About us" page; gives the page's id.
async fn site_with_a_page() -> (TestSite, i64) {
    let mut site = TestSite::with_owner().await;
    for title in ["First post", "Second post"] {
        site.post(
            "/admin/posts",
            &[
                ("title", title),
                ("content", "Text."),
                ("status", "published"),
            ],
        )
        .await;
    }
    site.post(
        "/admin/pages",
        &[
            ("title", "About us"),
            ("content", "We bake bread."),
            ("status", "published"),
        ],
    )
    .await;
    let id = newest_id(&site).await;
    (site, id)
}

#[tokio::test]
async fn a_page_can_be_the_home_page_and_the_posts_move_to_blog() {
    let (mut site, page_id) = site_with_a_page().await;
    let saved = choose_home_page(&mut site, &page_id.to_string()).await;
    assert!(saved.body.contains("Saved."), "{}", saved.body);

    // The admin marks it and suggests a menu link to the posts.
    let pages = site.get("/admin/pages").await;
    assert!(pages.body.contains("badge-home"));
    let settings = site.get("/admin/settings").await;
    assert!(settings.body.contains(r#"value="/blog""#));
    assert!(settings.body.contains("Your posts are at /blog."));

    site.new_visitor();
    let home = site.get("/").await;
    assert_eq!(home.status, StatusCode::OK);
    assert!(home.body.contains("We bake bread."));
    // Search engines see it at the site's address, not its own.
    let start = home.body.find(r#"<link rel="canonical" href=""#).unwrap() + 28;
    let canonical = &home.body[start..start + home.body[start..].find('"').unwrap()];
    assert!(
        canonical.ends_with('/') && !canonical.contains("about-us"),
        "{canonical}"
    );

    // The page has one address: its own sends visitors to the home page.
    let own = site.get("/about-us").await;
    assert_eq!(own.status, StatusCode::TEMPORARY_REDIRECT);
    assert_eq!(own.location(), "/");

    // The posts, one per page here, with their own pages.
    let blog = site.get("/blog").await;
    assert_eq!(blog.status, StatusCode::OK);
    // Tera writes "/" in links as "&#x2F;".
    assert!(blog.body.contains("&#x2F;blog?page=2"));
    let both = blog.body + &site.get("/blog?page=2").await.body;
    assert!(both.contains("First post") && both.contains("Second post"));
    let old_link = site.get("/?page=2").await;
    assert_eq!(old_link.location(), "/blog?page=2");

    let sitemap = site.get("/sitemap.xml").await.body;
    assert!(sitemap.contains("/blog</loc>"));
    assert!(!sitemap.contains("/about-us</loc>"));
}

#[tokio::test]
async fn the_latest_posts_come_back_when_the_home_page_goes_to_the_trash() {
    let (mut site, page_id) = site_with_a_page().await;
    choose_home_page(&mut site, &page_id.to_string()).await;
    site.post(&format!("/admin/pages/{page_id}/trash"), &[])
        .await;

    site.new_visitor();
    let home = site.get("/").await;
    assert!(home.body.contains("First post") || home.body.contains("Second post"));
    assert!(!home.body.contains("We bake bread."));
    assert_eq!(site.get("/blog").await.location(), "/");
}

#[tokio::test]
async fn without_a_home_page_nothing_changes() {
    let (mut site, _) = site_with_a_page().await;
    site.new_visitor();
    let home = site.get("/").await.body;
    assert!(home.contains("First post") || home.contains("Second post"));
    assert_eq!(site.get("/about-us").await.status, StatusCode::OK);
    assert_eq!(site.get("/blog").await.location(), "/");
}

#[tokio::test]
async fn a_page_already_at_blog_must_move_first() {
    let (mut site, page_id) = site_with_a_page().await;
    // Pages made before 1.3 could use the address /blog.
    sqlx::query("UPDATE posts SET slug = 'blog' WHERE id = ?")
        .bind(page_id)
        .execute(&site.pool)
        .await
        .unwrap();
    let refused = choose_home_page(&mut site, &page_id.to_string()).await;
    assert!(refused
        .body
        .contains("You have a page at /blog. Change its address first"));
    // And it still opens at its address.
    site.new_visitor();
    assert!(site.get("/blog").await.body.contains("We bake bread."));
}

#[tokio::test]
async fn new_pages_cant_use_the_address_blog() {
    let mut site = TestSite::with_owner().await;
    site.post(
        "/admin/pages",
        &[
            ("title", "Blog"),
            ("content", "Hi."),
            ("status", "published"),
        ],
    )
    .await;
    let slug: String = sqlx::query_scalar("SELECT slug FROM posts")
        .fetch_one(&site.pool)
        .await
        .unwrap();
    assert_ne!(slug, "blog");
}

#[tokio::test]
async fn only_published_pages_can_be_the_home_page() {
    let (mut site, _) = site_with_a_page().await;
    let post_id: i64 = sqlx::query_scalar("SELECT MIN(id) FROM posts")
        .fetch_one(&site.pool)
        .await
        .unwrap();
    for wrong in [post_id.to_string(), "9999".to_string(), "about".to_string()] {
        let refused = choose_home_page(&mut site, &wrong).await;
        assert!(
            refused.body.contains("Choose a page from the list."),
            "{wrong}"
        );
    }
}
