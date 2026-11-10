//! The Dashboard's boxes: each person's own order, and hiding the ones they
//! don't need (Customize).

use super::TestSite;
use axum::http::StatusCode;

/// The boxes on the Dashboard, top to bottom, with `-` for hidden ones.
fn boxes(page: &str) -> Vec<String> {
    page.split(r#"<div class="dashboard-section" data-section=""#)
        .skip(1)
        .map(|rest| {
            let id = &rest[..rest.find('"').unwrap()];
            let tag = &rest[..rest.find('>').unwrap()];
            if tag.ends_with("hidden") {
                format!("-{id}")
            } else {
                id.to_string()
            }
        })
        .collect()
}

#[tokio::test]
async fn the_dashboard_starts_in_the_usual_order_without_a_new_post_button() {
    let mut site = TestSite::with_owner().await;
    let page = site.get("/admin").await.body;
    assert_eq!(
        boxes(&page),
        ["health", "stats", "views", "referrers", "top_posts"]
    );
    assert!(page.contains("data-dashboard-customize"));
    assert!(!page.contains(r#"<a href="/admin/posts/new" class="btn btn-primary">"#));
}

#[tokio::test]
async fn each_person_keeps_their_own_order_and_hidden_boxes() {
    let mut site = TestSite::with_owner().await;
    let saved = site
        .post(
            "/admin/dashboard/layout",
            &[("layout", "top_posts,-stats,views,not-a-box,views")],
        )
        .await;
    assert_eq!(saved.status, StatusCode::NO_CONTENT);
    // Unknown and repeated boxes are dropped; missing ones come at the end.
    let stored: String = sqlx::query_scalar("SELECT dashboard_layout FROM users WHERE id = 1")
        .fetch_one(&site.pool)
        .await
        .unwrap();
    assert_eq!(stored, "top_posts,-stats,views,health,referrers");
    assert_eq!(
        boxes(&site.get("/admin").await.body),
        ["top_posts", "-stats", "views", "health", "referrers"]
    );

    // An author has no Site health box, whatever they send.
    let password = "Author-Pass-123!";
    let hash = crate::app::security::hash_password(password).unwrap();
    sqlx::query("INSERT INTO users (email, password_hash, name, role) VALUES ('author@example.com', ?, 'Author', 'author')")
        .bind(hash)
        .execute(&site.pool)
        .await
        .unwrap();
    site.new_visitor();
    site.sign_in("author@example.com", password).await;
    assert_eq!(
        boxes(&site.get("/admin").await.body),
        ["stats", "views", "referrers", "top_posts"]
    );
    site.post("/admin/dashboard/layout", &[("layout", "health,referrers")])
        .await;
    assert_eq!(
        boxes(&site.get("/admin").await.body),
        ["referrers", "stats", "views", "top_posts"]
    );
}
