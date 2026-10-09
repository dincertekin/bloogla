//! Saving settings and your profile without reloading the page by hand.

use super::{TestSite, ADMIN_EMAIL};

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
