//! The numbers next to menu items, and the "new version" notice.

use super::TestSite;
use crate::services::updates::Release;

#[tokio::test]
async fn the_menu_counts_comments_waiting_for_approval() {
    let mut site = TestSite::with_owner().await;
    let badge = r#"id="badge-comments" data-live hidden"#;
    assert!(
        site.get("/admin").await.body.contains(badge),
        "hidden with none waiting"
    );

    site.publish("Talk To Me", "published").await;
    for text in ["First!", "Second!"] {
        sqlx::query(
            "INSERT INTO comments (post_id, author_name, content, status)
             SELECT id, 'Reader', ?, 'pending' FROM posts LIMIT 1",
        )
        .bind(text)
        .execute(&site.pool)
        .await
        .unwrap();
    }
    let page = site.get("/admin/posts").await.body;
    assert!(!page.contains(badge), "shown on every admin page");
    assert!(page.contains(r#"<span aria-hidden="true">2</span>"#));
}

#[tokio::test]
async fn admins_hear_about_a_new_version() {
    let mut site = TestSite::with_owner().await;
    assert!(!site
        .get("/admin")
        .await
        .body
        .contains("is available. You have"));

    // What the daily check stores when GitHub has a newer release.
    *site.newer_release.write().unwrap() = Some(Release {
        version: "9.0.0".into(),
        url: "https://github.com/dincertekin/bloogla/releases/tag/v9.0.0".into(),
        download: None,
    });
    let dashboard = site.get("/admin").await.body;
    assert!(dashboard.contains("Bloogla 9.0.0 is available."));
    assert!(dashboard.contains("/releases/tag/v9.0.0"));
    assert!(
        !dashboard.contains(r#"id="badge-update" data-live hidden"#),
        "menu badge shown"
    );

    let settings = site.get("/admin/settings").await.body;
    assert!(settings.contains("Bloogla 9.0.0 is available."));
    assert!(settings.contains("Check for updates"));
}
