//! First-run setup, signing in and out, and resetting a forgotten password.

use super::{TestSite, ADMIN_EMAIL, ADMIN_PASSWORD, SETUP_CODE};
use axum::http::StatusCode;

#[tokio::test]
async fn a_new_site_sends_everyone_to_setup() {
    let mut site = TestSite::new().await;
    for path in ["/", "/admin", "/post/anything"] {
        let page = site.get(path).await;
        assert_eq!(page.location(), "/setup", "{path} should go to setup");
    }
    assert_eq!(site.get("/setup").await.status, StatusCode::OK);
}

#[tokio::test]
async fn setup_needs_the_code_and_a_strong_password() {
    let mut site = TestSite::new().await;

    let wrong_code = site.finish_setup("not-the-code", ADMIN_PASSWORD).await;
    assert!(
        wrong_code.body.contains("That setup code isn"),
        "says the code is wrong"
    );

    let weak = site.finish_setup(SETUP_CODE, "short").await;
    assert_eq!(weak.status, StatusCode::OK, "the form is shown again");
    let accounts: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM users")
        .fetch_one(&site.pool)
        .await
        .unwrap();
    assert_eq!(accounts, 0, "no account for a weak password");

    let done = site.finish_setup(SETUP_CODE, ADMIN_PASSWORD).await;
    assert_eq!(done.location(), "/admin");
    // The owner is signed in and welcomed once; setup can't run twice.
    let dashboard = site.get("/admin").await;
    assert_eq!(dashboard.status, StatusCode::OK);
    assert!(dashboard.body.contains("Your site is ready."));
    assert!(!site
        .get("/admin")
        .await
        .body
        .contains("Your site is ready."));
    assert!(!site
        .get("/admin?welcome=1")
        .await
        .body
        .contains("Your site is ready."));
    assert_eq!(site.get("/setup").await.location(), "/admin");
}

#[tokio::test]
async fn admin_pages_need_signing_in() {
    let mut site = TestSite::with_owner().await;
    site.new_visitor();

    assert_eq!(site.get("/admin/posts").await.location(), "/admin/login");
    let post = site.post("/admin/posts", &[("title", "Sneaky")]).await;
    assert_eq!(post.status, StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn signing_in_and_out() {
    let mut site = TestSite::with_owner().await;
    site.get("/admin/logout").await;
    assert_eq!(site.get("/admin").await.location(), "/admin/login");

    let wrong = site.sign_in(ADMIN_EMAIL, "Wrong-Password-1").await;
    assert!(wrong.body.contains("Invalid email or password."));

    let right = site.sign_in(ADMIN_EMAIL, ADMIN_PASSWORD).await;
    assert_eq!(right.location(), "/admin");
    assert_eq!(site.get("/admin").await.status, StatusCode::OK);
}

#[tokio::test]
async fn forgot_password_explains_what_to_do_without_email() {
    let mut site = TestSite::with_owner().await;
    site.new_visitor();
    let page = site.get("/admin/forgot-password").await;
    assert!(page.body.contains("bloogla reset-password"));
    assert!(site
        .get("/admin/login")
        .await
        .body
        .contains("/admin/forgot-password"));
}

#[tokio::test]
async fn a_reset_link_sets_a_new_password_once() {
    let mut site = TestSite::with_owner().await;
    site.new_visitor();
    let user_id: i64 = sqlx::query_scalar("SELECT id FROM users")
        .fetch_one(&site.pool)
        .await
        .unwrap();
    // The code that the email would contain.
    let token = crate::handlers::admin::password_reset::create_link(&site.pool, user_id)
        .await
        .unwrap();
    let link = format!("/admin/reset-password?token={token}");

    let form = site.get(&link).await;
    assert!(form.body.contains("name=\"token\""));
    assert_eq!(form.header("cache-control"), "no-store");
    assert!(site
        .get("/admin/reset-password?token=nope")
        .await
        .body
        .contains("/admin/forgot-password"));

    let new_password = "Another-Good-Pass-7";
    let saved = site
        .post(
            "/admin/reset-password",
            &[
                ("token", token.as_str()),
                ("password", new_password),
                ("confirm_password", new_password),
            ],
        )
        .await;
    assert_eq!(saved.location(), "/admin/login?reset=1");

    // The link is used up, the old password stopped working, the new one works.
    assert!(!site.get(&link).await.body.contains("name=\"token\""));
    let old = site.sign_in(ADMIN_EMAIL, ADMIN_PASSWORD).await;
    assert!(old.body.contains("Invalid email or password."));
    assert_eq!(
        site.sign_in(ADMIN_EMAIL, new_password).await.location(),
        "/admin"
    );
}

#[tokio::test]
async fn every_admin_screen_opens() {
    let mut site = TestSite::with_owner().await;
    for path in [
        "/admin",
        "/admin/posts",
        "/admin/posts/new",
        "/admin/pages",
        "/admin/media",
        "/admin/comments",
        "/admin/tags",
        "/admin/themes/default/options",
        "/admin/users",
        "/admin/profile",
        "/admin/settings",
    ] {
        assert_eq!(site.get(path).await.status, StatusCode::OK, "{path}");
    }

    // "Appearance" lists the bundled themes to choose from.
    let themes = site.get("/admin/themes").await;
    assert_eq!(themes.status, StatusCode::OK);
    for name in ["Default Theme", "Story", "Gazette"] {
        assert!(themes.body.contains(name), "{name} is listed");
    }
    for id in ["default", "story", "gazette"] {
        let preview = format!("/theme-assets/{id}/static/preview.svg");
        assert!(themes.body.contains(&preview), "{id} has a preview");
    }
    for id in ["story", "gazette"] {
        let path = format!("/admin/themes/{id}/options");
        assert_eq!(site.get(&path).await.status, StatusCode::OK, "{path}");
    }

    let dashboard = site.get("/admin").await;
    assert!(dashboard.body.contains("Get your site ready"));
    site.post("/admin/checklist/hide", &[]).await;
    assert!(!site
        .get("/admin")
        .await
        .body
        .contains("Get your site ready"));
}
