//! Saving settings and your profile without reloading the page by hand.

use super::{TestSite, ADMIN_EMAIL, ADMIN_PASSWORD};
use axum::http::StatusCode;

async fn email_now(site: &TestSite) -> String {
    sqlx::query_scalar("SELECT email FROM users")
        .fetch_one(&site.pool)
        .await
        .unwrap()
}

#[tokio::test]
async fn changing_your_email_needs_your_password() {
    let mut site = TestSite::with_owner().await;
    let new_email = "someone-else@example.com";

    let without = site
        .post("/admin/profile", &[("name", "Owner"), ("email", new_email)])
        .await;
    assert!(without
        .body
        .contains("Enter your current password to change your email."));
    let wrong = site
        .post(
            "/admin/profile",
            &[
                ("name", "Owner"),
                ("email", new_email),
                ("current_password", "wrong"),
            ],
        )
        .await;
    assert_eq!(wrong.status, StatusCode::UNPROCESSABLE_ENTITY);
    assert!(wrong.body.contains("Your current password isn't right."));
    assert_eq!(email_now(&site).await, ADMIN_EMAIL);

    let name_only = site
        .post("/admin/profile", &[("name", "Sam"), ("email", ADMIN_EMAIL)])
        .await;
    assert!(name_only.body.contains("Saved."));

    let right = site
        .post(
            "/admin/profile",
            &[
                ("name", "Sam"),
                ("email", new_email),
                ("current_password", ADMIN_PASSWORD),
            ],
        )
        .await;
    assert!(right.body.contains("Saved."));
    assert_eq!(email_now(&site).await, new_email);
}

#[tokio::test]
async fn a_new_site_language_reloads_the_admin_in_that_language() {
    let mut site = TestSite::with_owner().await;

    // The owner follows the site's language, so the whole page must change.
    let saved = site
        .post(
            "/admin/settings/general",
            &[("blog_name", "Test Blog"), ("language", "tr")],
        )
        .await;
    assert_eq!(saved.header("hx-trigger"), "reload-page");
    assert!(
        saved.body.contains("Kaydedildi."),
        "message in the new language"
    );
    assert!(site
        .get("/admin/settings")
        .await
        .body
        .contains("<html lang=\"tr\">"));

    // Saving without changing the language just says so, no reload.
    let again = site
        .post(
            "/admin/settings/general",
            &[("blog_name", "Test Blog"), ("language", "tr")],
        )
        .await;
    assert_eq!(again.header("hx-trigger"), "");
}

#[tokio::test]
async fn people_with_their_own_language_keep_it() {
    let mut site = TestSite::with_owner().await;
    let profile = site
        .post(
            "/admin/profile",
            &[
                ("name", "Owner"),
                ("email", ADMIN_EMAIL),
                ("language", "en"),
            ],
        )
        .await;
    assert_eq!(profile.header("hx-trigger"), "reload-page");

    // A new site language doesn't change their admin panel.
    let saved = site
        .post(
            "/admin/settings/general",
            &[("blog_name", "Test Blog"), ("language", "tr")],
        )
        .await;
    assert_eq!(saved.header("hx-trigger"), "");
    assert!(saved.body.contains("Saved."));
}

#[tokio::test]
async fn daily_backups_can_be_turned_off_and_on() {
    let mut site = TestSite::with_owner().await;
    let checkbox = r#"name="auto_backup" value="1" checked"#;
    // On for a new site.
    assert!(site.get("/admin/settings").await.body.contains(checkbox));

    // An unticked checkbox isn't sent, so an empty form turns it off.
    let saved = site.post("/admin/settings/backup", &[]).await;
    assert!(saved.body.contains("Saved."));
    assert!(!site.get("/admin/settings").await.body.contains(checkbox));

    site.post("/admin/settings/backup", &[("auto_backup", "1")])
        .await;
    assert!(site.get("/admin/settings").await.body.contains(checkbox));
}

#[tokio::test]
async fn sites_that_turned_backups_off_in_1_0_keep_them_off() {
    let mut site = TestSite::with_owner().await;
    let checkbox = r#"name="auto_backup" value="1" checked"#;

    // `BLOOGLA_AUTO_BACKUP=false` from Bloogla 1.0 becomes the unticked box.
    crate::services::backup::keep_old_off_switch(&site.pool, Some("false".into())).await;
    assert!(!site.get("/admin/settings").await.body.contains(checkbox));

    // Once it's ticked in the admin, the old variable no longer matters.
    site.post("/admin/settings/backup", &[("auto_backup", "1")])
        .await;
    crate::services::backup::keep_old_off_switch(&site.pool, Some("false".into())).await;
    assert!(site.get("/admin/settings").await.body.contains(checkbox));
}

#[tokio::test]
async fn backups_stay_on_without_the_old_variable() {
    let mut site = TestSite::with_owner().await;
    crate::services::backup::keep_old_off_switch(&site.pool, None).await;
    crate::services::backup::keep_old_off_switch(&site.pool, Some("true".into())).await;
    let page = site.get("/admin/settings").await;
    assert!(page
        .body
        .contains(r#"name="auto_backup" value="1" checked"#));
}
