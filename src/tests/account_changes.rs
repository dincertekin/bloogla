//! Changes to how someone signs in (email, password, two-factor login) ask
//! for the password, sign other devices out and cancel older reset links.
//! (From pull requests #18, #21 and #26 by Rashandd and enstalha.)

use super::{TestSite, ADMIN_EMAIL, ADMIN_PASSWORD};
use axum::http::StatusCode;

const NEW_EMAIL: &str = "someone-else@example.com";

async fn email_now(site: &TestSite) -> String {
    sqlx::query_scalar("SELECT email FROM users")
        .fetch_one(&site.pool)
        .await
        .unwrap()
}

/// A reset link for the owner, as the email would contain it.
async fn reset_link(site: &TestSite) -> String {
    let user_id: i64 = sqlx::query_scalar("SELECT id FROM users")
        .fetch_one(&site.pool)
        .await
        .unwrap();
    let token = crate::handlers::admin::password_reset::create_link(&site.pool, user_id)
        .await
        .unwrap();
    format!("/admin/reset-password?token={token}")
}

/// Whether a reset link still opens its form.
async fn link_works(site: &mut TestSite, link: &str) -> bool {
    site.get(link).await.body.contains("name=\"token\"")
}

/// Sign the owner in on a second device and give back its cookie; the site
/// goes on as the first device.
async fn second_device(site: &mut TestSite) -> Option<String> {
    let first = site.cookie.clone();
    site.new_visitor();
    site.sign_in(ADMIN_EMAIL, ADMIN_PASSWORD).await;
    let second = site.cookie.clone();
    site.cookie = first;
    second
}

/// Whether a device with `cookie` is still signed in.
async fn signed_in(site: &mut TestSite, cookie: &Option<String>) -> bool {
    let mine = std::mem::replace(&mut site.cookie, cookie.clone());
    let page = site.get("/admin").await;
    site.cookie = mine;
    page.status == StatusCode::OK
}

/// Turn two-factor login on directly in the database, with a known key.
async fn turn_on_two_factor(site: &TestSite) -> Vec<u8> {
    crate::app::secrets::use_test_key();
    let secret = b"12345678901234567890".to_vec();
    sqlx::query("UPDATE users SET totp_secret = ?")
        .bind(crate::app::secrets::encrypt(&crate::app::security::to_hex(
            &secret,
        )))
        .execute(&site.pool)
        .await
        .unwrap();
    secret
}

#[tokio::test]
async fn changing_your_email_needs_your_password() {
    let mut site = TestSite::with_owner().await;
    let profile = site.get("/admin/profile").await.body;
    assert!(profile.contains(r#"<label for="email_current_password">Current password</label>"#));

    let without = site
        .post("/admin/profile", &[("name", "Owner"), ("email", NEW_EMAIL)])
        .await;
    assert_eq!(without.status, StatusCode::UNPROCESSABLE_ENTITY);
    assert!(without
        .body
        .contains("Enter your current password to change your email."));
    let wrong = site
        .post(
            "/admin/profile",
            &[
                ("name", "Owner"),
                ("email", NEW_EMAIL),
                ("current_password", "wrong"),
            ],
        )
        .await;
    assert_eq!(wrong.status, StatusCode::UNPROCESSABLE_ENTITY);
    assert!(wrong.body.contains("Your current password isn't right."));
    assert_eq!(email_now(&site).await, ADMIN_EMAIL);

    // Only the name: no password needed (the email's case doesn't count).
    let name_only = site
        .post(
            "/admin/profile",
            &[("name", "Sam"), ("email", "Owner@Example.com")],
        )
        .await;
    assert!(name_only.body.contains("Saved."));
}

#[tokio::test]
async fn a_new_email_signs_out_other_devices_and_cancels_reset_links() {
    let mut site = TestSite::with_owner().await;
    let other = second_device(&mut site).await;
    let link = reset_link(&site).await;

    let right = site
        .post(
            "/admin/profile",
            &[
                ("name", "Owner"),
                ("email", NEW_EMAIL),
                ("current_password", ADMIN_PASSWORD),
            ],
        )
        .await;
    assert_eq!(right.status, StatusCode::OK);
    assert_eq!(email_now(&site).await, NEW_EMAIL);
    // The page reloads, so the password field is emptied.
    assert_eq!(right.header("hx-trigger"), "reload-page");

    let mine = site.cookie.clone();
    assert!(signed_in(&mut site, &mine).await);
    assert!(!signed_in(&mut site, &other).await);
    assert!(!link_works(&mut site, &link).await);
}

#[tokio::test]
async fn a_new_password_cancels_reset_links() {
    let mut site = TestSite::with_owner().await;
    let link = reset_link(&site).await;
    let new_password = "Replacement-Password-7";
    let changed = site
        .post(
            "/admin/profile/password",
            &[
                ("current_password", ADMIN_PASSWORD),
                ("new_password", new_password),
                ("confirm_password", new_password),
            ],
        )
        .await;
    assert!(changed.body.contains("Password updated."));
    assert!(!link_works(&mut site, &link).await);
}

#[tokio::test]
async fn a_used_reset_link_cant_set_a_password_again() {
    let mut site = TestSite::with_owner().await;
    site.new_visitor();
    let link = reset_link(&site).await;
    let token = link.split("token=").nth(1).unwrap().to_string();
    let reset = |password: &'static str| {
        vec![
            ("token", token.clone()),
            ("password", password.to_string()),
            ("confirm_password", password.to_string()),
        ]
    };
    let first = reset("Another-Good-Pass-7");
    let fields: Vec<(&str, &str)> = first.iter().map(|(k, v)| (*k, v.as_str())).collect();
    assert_eq!(
        site.post("/admin/reset-password", &fields).await.location(),
        "/admin/login?reset=1"
    );

    let second = reset("Third-Good-Pass-8");
    let fields: Vec<(&str, &str)> = second.iter().map(|(k, v)| (*k, v.as_str())).collect();
    let again = site.post("/admin/reset-password", &fields).await;
    assert!(again.body.contains("/admin/forgot-password"));
    assert_eq!(
        site.sign_in(ADMIN_EMAIL, "Another-Good-Pass-7")
            .await
            .location(),
        "/admin"
    );
}

#[tokio::test]
async fn a_password_change_cancels_a_sign_in_waiting_for_its_code() {
    let mut site = TestSite::with_owner().await;
    let secret = turn_on_two_factor(&site).await;
    let owner = site.cookie.clone();

    // Someone else has the old password and reaches the code step.
    site.new_visitor();
    assert_eq!(
        site.sign_in(ADMIN_EMAIL, ADMIN_PASSWORD).await.location(),
        "/admin/login/code"
    );
    let waiting = site.cookie.clone();

    // The owner changes the password meanwhile.
    site.cookie = owner;
    let new_password = "Replacement-Password-7";
    site.post(
        "/admin/profile/password",
        &[
            ("current_password", ADMIN_PASSWORD),
            ("new_password", new_password),
            ("confirm_password", new_password),
        ],
    )
    .await;

    // The old password's check no longer counts, even with the right code.
    site.cookie = waiting;
    let code = crate::app::totp::code_now(&secret);
    let page = site.post("/admin/login/code", &[("code", &code)]).await;
    assert_eq!(page.location(), "/admin/login");
    assert_ne!(site.get("/admin").await.status, StatusCode::OK);
}

/// The Base32 key from the setup page, back into bytes.
fn key_from_page(page: &str) -> Vec<u8> {
    let start = page.find(r#"class="two-factor-key">"#).unwrap() + 23;
    let text = &page[start..start + page[start..].find('<').unwrap()];
    let mut bits: u64 = 0;
    let mut count = 0;
    let mut bytes = Vec::new();
    for c in text.chars().filter(|c| !c.is_whitespace()) {
        let value = "ABCDEFGHIJKLMNOPQRSTUVWXYZ234567".find(c).unwrap() as u64;
        bits = (bits << 5) | value;
        count += 5;
        if count >= 8 {
            count -= 8;
            bytes.push((bits >> count) as u8);
        }
    }
    bytes
}

#[tokio::test]
async fn turning_on_two_factor_needs_your_password() {
    let mut site = TestSite::with_owner().await;
    crate::app::secrets::use_test_key();
    let other = second_device(&mut site).await;
    let setup = site.get("/admin/profile/two-factor").await.body;
    assert!(setup.contains(r#"<label for="two-factor-password">Current password</label>"#));
    let secret = key_from_page(&setup);
    let code = crate::app::totp::code_now(&secret);

    let wrong = site
        .post(
            "/admin/profile/two-factor",
            &[("password", "wrong"), ("code", &code)],
        )
        .await;
    assert_eq!(wrong.status, StatusCode::UNPROCESSABLE_ENTITY);
    // (The page writes the apostrophe as an HTML code.)
    assert!(wrong.body.contains("Your current password isn"));
    assert!(crate::db::users::two_factor(&site.pool, 1).await.is_none());

    let right = site
        .post(
            "/admin/profile/two-factor",
            &[("password", ADMIN_PASSWORD), ("code", &code)],
        )
        .await;
    assert_eq!(right.location(), "/admin/profile/two-factor/codes");
    let codes = site.get("/admin/profile/two-factor/codes").await;
    assert!(codes.body.contains("Save your recovery codes"));
    // Shown once.
    assert_eq!(
        site.get("/admin/profile/two-factor/codes").await.location(),
        "/admin/profile#two-factor"
    );
    // The other device signed in without a code, so it's signed out.
    assert!(!signed_in(&mut site, &other).await);
}

#[tokio::test]
async fn new_recovery_codes_need_your_password() {
    let mut site = TestSite::with_owner().await;
    turn_on_two_factor(&site).await;
    let profile = site.get("/admin/profile").await.body;
    assert!(profile.contains(r#"<label for="recovery-codes-password">Current password</label>"#));
    let other = second_device(&mut site).await;

    let wrong = site
        .post(
            "/admin/profile/two-factor/recovery",
            &[("password", "wrong")],
        )
        .await;
    assert_eq!(wrong.status, StatusCode::UNPROCESSABLE_ENTITY);
    assert!(wrong.body.contains("Your current password isn't right."));

    let right = site
        .post(
            "/admin/profile/two-factor/recovery",
            &[("password", ADMIN_PASSWORD)],
        )
        .await;
    assert_eq!(
        right.header("hx-redirect"),
        "/admin/profile/two-factor/codes"
    );
    let codes = site.get("/admin/profile/two-factor/codes").await;
    assert!(codes.body.contains("Save your recovery codes"));
    assert!(!signed_in(&mut site, &other).await);
}
