//! The Dashboard's "Site health" card: what's fine and what to fix.

use super::TestSite;

#[tokio::test]
async fn site_health_says_what_to_fix() {
    let mut site = TestSite::with_owner().await;
    let dashboard = site.get("/admin").await.body;
    assert!(dashboard.contains("Site health"));
    assert!(dashboard.contains("so you won&#x27;t hear about new comments"));
    assert!(dashboard.contains("Checking for new versions is off."));
    // Tried out on this computer: HTTPS isn't needed, so it isn't mentioned.
    assert!(!dashboard.contains("HTTPS"));

    crate::db::settings::save(
        &site.pool,
        &[
            ("smtp_host", "mail.example.com".into()),
            ("auto_backup", "false".into()),
        ],
    )
    .await
    .unwrap();
    let dashboard = site.get("/admin").await.body;
    assert!(dashboard.contains("Email is set up."));
    assert!(dashboard.contains("Daily backups are off."));
    assert!(dashboard.contains("/admin/settings#backup"));
}

#[tokio::test]
async fn only_admins_see_site_health() {
    let mut site = TestSite::with_owner().await;
    let password = "Author-Pass-123!";
    let hash = crate::app::security::hash_password(password).unwrap();
    sqlx::query("INSERT INTO users (email, password_hash, name, role) VALUES ('author@example.com', ?, 'Author', 'author')")
        .bind(hash)
        .execute(&site.pool)
        .await
        .unwrap();
    site.new_visitor();
    site.sign_in("author@example.com", password).await;
    let dashboard = site.get("/admin").await.body;
    assert!(dashboard.contains("Overview"), "signed in as the author");
    assert!(!dashboard.contains("Site health"));
}
