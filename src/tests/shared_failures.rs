//! Completed failures share one account limit across sign-in and profile forms.
use super::{TestSite, ADMIN_EMAIL, ADMIN_PASSWORD};
use axum::http::StatusCode;

async fn wrong_password(site: &mut TestSite, path: &str) -> super::Page {
    if path.ends_with("/off") {
        site.post(path, &[("password", "wrong")]).await
    } else {
        site.post(
            path,
            &[
                ("current_password", "wrong"),
                ("new_password", "Replacement-Password-7"),
                ("confirm_password", "Replacement-Password-7"),
            ],
        )
        .await
    }
}

#[tokio::test]
async fn profile_failures_slow_sign_in_across_forms_and_email_case() {
    let mut site = TestSite::with_owner().await;
    sqlx::query("UPDATE users SET email = 'OWNER@EXAMPLE.COM'")
        .execute(&site.pool)
        .await
        .unwrap();
    for path in [
        "/admin/profile/password",
        "/admin/profile/two-factor/off",
        "/admin/profile/password",
        "/admin/profile/two-factor/off",
        "/admin/profile/password",
    ] {
        assert_eq!(
            wrong_password(&mut site, path).await.status,
            StatusCode::UNPROCESSABLE_ENTITY
        );
    }
    let result = site.sign_in(ADMIN_EMAIL, ADMIN_PASSWORD).await;
    assert_eq!(result.status, StatusCode::TOO_MANY_REQUESTS);
    assert!(result.body.contains("Too many failed sign-ins"));
}

#[tokio::test]
async fn sign_in_failures_slow_profile_forms_with_visible_html() {
    let mut site = TestSite::with_owner().await;
    let cookie = site.cookie.clone();
    site.new_visitor();
    for _ in 0..5 {
        site.sign_in(ADMIN_EMAIL, "wrong").await;
    }
    site.cookie = cookie;
    for path in ["/admin/profile/password", "/admin/profile/two-factor/off"] {
        let result = wrong_password(&mut site, path).await;
        assert_eq!(result.status, StatusCode::TOO_MANY_REQUESTS);
        assert!(result.header("content-type").starts_with("text/html"));
        assert!(result.body.contains("Too many failed sign-ins"));
    }
}

#[tokio::test]
async fn a_successful_confirmation_preserves_failures_from_other_forms() {
    let mut site = TestSite::with_owner().await;
    for _ in 0..4 {
        assert_eq!(
            wrong_password(&mut site, "/admin/profile/password")
                .await
                .status,
            StatusCode::UNPROCESSABLE_ENTITY
        );
    }
    assert_eq!(
        site.post(
            "/admin/profile/two-factor/off",
            &[("password", ADMIN_PASSWORD)]
        )
        .await
        .status,
        StatusCode::OK
    );
    assert_eq!(
        wrong_password(&mut site, "/admin/profile/password")
            .await
            .status,
        StatusCode::UNPROCESSABLE_ENTITY
    );
    assert_eq!(
        wrong_password(&mut site, "/admin/profile/password")
            .await
            .status,
        StatusCode::TOO_MANY_REQUESTS
    );
}

#[tokio::test]
async fn a_correct_password_does_not_clear_failed_two_factor_guesses() {
    let mut site = TestSite::with_owner().await;
    let secret = b"12345678901234567890";
    sqlx::query("UPDATE users SET totp_secret = ?")
        .bind(crate::app::security::to_hex(secret))
        .execute(&site.pool)
        .await
        .unwrap();
    site.new_visitor();
    assert_eq!(
        site.sign_in(ADMIN_EMAIL, ADMIN_PASSWORD).await.location(),
        "/admin/login/code"
    );
    let now = chrono::Utc::now().timestamp() as u64;
    let wrong = (0..10)
        .map(|n| format!("{n:06}"))
        .find(|code| crate::app::totp::verify(secret, code, now, 0).is_none())
        .unwrap();
    for _ in 0..4 {
        assert_eq!(
            site.post("/admin/login/code", &[("code", &wrong)])
                .await
                .status,
            StatusCode::OK
        );
    }
    site.new_visitor();
    assert_eq!(
        site.sign_in(ADMIN_EMAIL, ADMIN_PASSWORD).await.location(),
        "/admin/login/code"
    );
    assert_eq!(
        site.post("/admin/login/code", &[("code", &wrong)])
            .await
            .status,
        StatusCode::OK
    );
    assert_eq!(
        site.post("/admin/login/code", &[("code", &wrong)])
            .await
            .status,
        StatusCode::TOO_MANY_REQUESTS
    );
}
