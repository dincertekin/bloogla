//! Saving settings and your profile without reloading the page by hand.

use super::{TestSite, ADMIN_EMAIL};
use axum::http::StatusCode;

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

#[tokio::test]
async fn each_person_chooses_light_or_dark_for_the_admin() {
    let mut site = TestSite::with_owner().await;
    // Like the device at first: no choice on <html>.
    assert!(site
        .get("/admin")
        .await
        .body
        .contains(r#"<html lang="en">"#));

    for (scheme, expected) in [
        ("dark", r#" data-scheme="dark""#),
        ("light", r#" data-scheme="light""#),
    ] {
        let saved = site
            .post("/admin/profile/color-scheme", &[("scheme", scheme)])
            .await;
        assert_eq!(saved.status, StatusCode::NO_CONTENT);
        let page = site.get("/admin").await.body;
        assert!(
            page.contains(&format!(r#"<html lang="en"{expected}>"#)),
            "{scheme}"
        );
        assert!(page.contains(&format!(r#"data-scheme-switch data-state="{scheme}""#)));
    }

    // Back to the device; something else is refused.
    site.post("/admin/profile/color-scheme", &[("scheme", "device")])
        .await;
    assert!(site
        .get("/admin")
        .await
        .body
        .contains(r#"<html lang="en">"#));
    let refused = site
        .post("/admin/profile/color-scheme", &[("scheme", "purple")])
        .await;
    assert_eq!(refused.status, StatusCode::UNPROCESSABLE_ENTITY);
}

#[tokio::test]
async fn verification_tags_go_on_every_page_and_scripts_dont() {
    let mut site = TestSite::with_owner().await;
    let page = site.get("/admin/settings").await.body;
    assert!(
        page.contains(r#"<label for="head_code">Code for search engines and other sites</label>"#)
    );

    let saved = site
        .post(
            "/admin/settings/general",
            &[
                ("blog_name", "Test Blog"),
                ("language", "en"),
                (
                    "head_code",
                    "<meta name=\"google-site-verification\" content=\"x7Gm\" />",
                ),
            ],
        )
        .await;
    assert!(saved.body.contains("Saved."), "{}", saved.body);
    let address = site.publish("Hello", "published").await;
    site.new_visitor();
    for path in ["/", address.as_str(), "/no-such-page"] {
        let page = site.get(path).await.body;
        assert!(
            page.contains(r#"<meta name="google-site-verification" content="x7Gm">"#),
            "{path}"
        );
    }

    site.sign_in(ADMIN_EMAIL, super::ADMIN_PASSWORD).await;
    let refused = site
        .post(
            "/admin/settings/general",
            &[
                ("blog_name", "Test Blog"),
                ("language", "en"),
                (
                    "head_code",
                    "<script src=\"https://tracker.example/t.js\"></script>",
                ),
            ],
        )
        .await;
    assert_eq!(refused.status, axum::http::StatusCode::UNPROCESSABLE_ENTITY);
    assert!(refused
        .body
        .contains("Only meta and link tags can go here."));
    // The tags saved before stay.
    site.new_visitor();
    let home = site.get("/").await.body;
    assert!(home.contains("google-site-verification"));
    assert!(!home.contains("tracker.example"));
}

#[tokio::test]
async fn backup_storage_is_saved_with_its_secret_locked() {
    let mut site = TestSite::with_owner().await;
    let fields = |endpoint: &'static str| {
        [
            ("storage_on", "1"),
            ("storage_endpoint", endpoint),
            ("storage_region", "eu-central-003"),
            ("storage_bucket", "my-backups"),
            ("storage_access_key", "key-id"),
            ("storage_secret_key", "very-secret"),
        ]
    };

    // Keys never travel without HTTPS.
    let refused = site
        .post("/admin/settings/storage", &fields("http://s3.example.com"))
        .await;
    assert_eq!(refused.status, StatusCode::UNPROCESSABLE_ENTITY);
    assert!(refused.body.contains("must start with https://"));
    assert!(!crate::db::settings::load(&site.pool).await.storage_on);

    // Saving off needs nothing else and checks nothing.
    let saved = site
        .post(
            "/admin/settings/storage",
            &[("storage_bucket", "my-backups")],
        )
        .await;
    assert!(saved.body.contains("Saved."), "{}", saved.body);

    // Saved on: the secret key is stored encrypted and read back.
    site.post(
        "/admin/settings/storage",
        &fields("https://storage.invalid"),
    )
    .await;
    let stored: String =
        sqlx::query_scalar("SELECT value FROM settings WHERE key = 'storage_secret_key'")
            .fetch_one(&site.pool)
            .await
            .unwrap();
    assert!(!stored.contains("very-secret"));
    let settings = crate::db::settings::load(&site.pool).await;
    assert!(settings.storage_on);
    assert_eq!(settings.storage_secret_key, "very-secret");
    // The address doesn't exist, so the check failed, and the Dashboard says so.
    assert!(!settings.storage_last_problem.is_empty());
    assert!(site
        .get("/admin")
        .await
        .body
        .contains("The last copy to your storage failed."));

    let page = site.get("/admin/settings").await.body;
    assert!(page.contains("Copy backups to storage"));
    assert!(
        !page.contains("very-secret"),
        "the secret isn't shown again"
    );
}
