//! Fresh password confirmation, shared guessing limits and recovery-address races.

use super::security_regressions::authenticator_code;
use super::{TestSite, ADMIN_EMAIL, ADMIN_PASSWORD};
use crate::db::users;
use crate::handlers::admin::password_reset::{create_link, create_link_if_current};
use axum::body::Body;
use axum::extract::ConnectInfo;
use axum::http::{header, Request, StatusCode};
use tower::ServiceExt;

async fn owner(site: &TestSite) -> crate::app::models::CurrentUser {
    let id = sqlx::query_scalar("SELECT id FROM users LIMIT 1")
        .fetch_one(&site.pool)
        .await
        .unwrap();
    users::find(&site.pool, id).await.unwrap()
}

async fn has_link(site: &TestSite, token: &str) -> bool {
    sqlx::query_scalar::<_, i64>("SELECT COUNT(*) FROM password_resets WHERE token_hash = ?")
        .bind(crate::app::security::hash_token(token))
        .fetch_one(&site.pool)
        .await
        .unwrap()
        == 1
}

async fn setup_key(site: &mut TestSite) -> Vec<u8> {
    let page = site.get("/admin/profile/two-factor").await;
    assert_eq!(page.status, StatusCode::OK);
    assert!(page.body.contains("name=\"password\""));
    let displayed = page
        .body
        .split("<code class=\"two-factor-key\">")
        .nth(1)
        .unwrap()
        .split("</code>")
        .next()
        .unwrap();
    let alphabet = "ABCDEFGHIJKLMNOPQRSTUVWXYZ234567";
    let mut secret = Vec::new();
    let (mut buffer, mut bits) = (0u32, 0);
    for character in displayed.chars().filter(|c| !c.is_whitespace()) {
        buffer = (buffer << 5) | alphabet.find(character).unwrap() as u32;
        bits += 5;
        if bits >= 8 {
            bits -= 8;
            secret.push((buffer >> bits) as u8);
        }
    }
    secret
}

#[tokio::test]
async fn email_changes_require_password_and_revoke_other_sessions_and_links() {
    let mut site = TestSite::with_owner().await;
    let me = owner(&site).await;
    let token = create_link(&site.pool, me.id).await.unwrap();
    let old_cookie = site.cookie.clone();
    site.new_visitor();
    assert_eq!(
        site.sign_in(ADMIN_EMAIL, ADMIN_PASSWORD).await.location(),
        "/admin"
    );
    assert!(site
        .get("/admin/profile")
        .await
        .body
        .contains("To change your email, enter your current password."));
    for password in [None, Some("wrong")] {
        let mut fields = vec![
            ("name", "Attacker"),
            ("email", "attacker@example.com"),
            ("language", "tr"),
        ];
        if let Some(password) = password {
            fields.push(("current_password", password));
        }
        let result = site.post("/admin/profile", &fields).await;
        assert_eq!(result.status, StatusCode::UNPROCESSABLE_ENTITY);
        let unchanged = owner(&site).await;
        assert_eq!(unchanged.email, me.email);
        assert_eq!(unchanged.name, me.name);
        assert_eq!(unchanged.session_version, me.session_version);
        assert_eq!(unchanged.lang.code(), "en");
        assert!(has_link(&site, &token).await);
    }
    let result = site
        .post(
            "/admin/profile",
            &[
                ("name", "Owner"),
                ("email", "new@example.com"),
                ("current_password", ADMIN_PASSWORD),
            ],
        )
        .await;
    assert_eq!(result.status, StatusCode::OK);
    let changed = owner(&site).await;
    assert_eq!(changed.email, "new@example.com");
    assert_eq!(changed.session_version, me.session_version + 1);
    assert!(!has_link(&site, &token).await);
    assert_eq!(site.get("/admin").await.status, StatusCode::OK);
    let new_cookie = site.cookie.clone();
    site.cookie = old_cookie;
    assert_eq!(site.get("/admin").await.location(), "/admin/login");
    site.cookie = new_cookie;
    let replacement = create_link(&site.pool, me.id).await.unwrap();
    assert!(
        create_link_if_current(&site.pool, me.id, me.session_version)
            .await
            .unwrap()
            .is_none()
    );
    assert!(
        has_link(&site, &replacement).await,
        "a stale old-address request must not delete the new link"
    );
    site.new_visitor();
    assert_eq!(
        site.sign_in("new@example.com", ADMIN_PASSWORD)
            .await
            .location(),
        "/admin"
    );
}

#[tokio::test]
async fn name_and_language_changes_do_not_require_password_or_revoke_access() {
    let mut site = TestSite::with_owner().await;
    let me = owner(&site).await;
    let token = create_link(&site.pool, me.id).await.unwrap();
    let result = site
        .post(
            "/admin/profile",
            &[
                ("name", "Renamed"),
                ("email", ADMIN_EMAIL),
                ("language", "tr"),
            ],
        )
        .await;
    assert_eq!(result.status, StatusCode::OK);
    let changed = owner(&site).await;
    assert_eq!(changed.name, "Renamed");
    assert_eq!(changed.lang.code(), "tr");
    assert_eq!(changed.session_version, me.session_version);
    assert!(has_link(&site, &token).await);
    assert_eq!(site.get("/admin").await.status, StatusCode::OK);
}

#[tokio::test]
async fn stale_profile_and_reset_requests_cannot_undo_a_password_change() {
    let site = TestSite::with_owner().await;
    let me = owner(&site).await;
    let hash = crate::app::security::hash_password("Replacement-Password-7").unwrap();
    users::set_password(&site.pool, me.id, &hash).await.unwrap();
    let new_link = create_link(&site.pool, me.id).await.unwrap();
    assert!(
        users::update_profile(&site.pool, &me, "Attacker", "attacker@example.com", "tr")
            .await
            .unwrap()
            .is_none()
    );
    assert!(
        create_link_if_current(&site.pool, me.id, me.session_version)
            .await
            .unwrap()
            .is_none()
    );
    assert!(has_link(&site, &new_link).await);
    assert_eq!(owner(&site).await.email, ADMIN_EMAIL);
}

#[tokio::test]
async fn email_changes_roll_back_if_link_revocation_fails() {
    let mut site = TestSite::with_owner().await;
    let me = owner(&site).await;
    let token = create_link(&site.pool, me.id).await.unwrap();
    sqlx::query("CREATE TRIGGER fail_revocation BEFORE DELETE ON password_resets BEGIN SELECT RAISE(ABORT, 'test failure'); END")
        .execute(&site.pool).await.unwrap();
    let result = site
        .post(
            "/admin/profile",
            &[
                ("name", "Changed"),
                ("email", "new@example.com"),
                ("current_password", ADMIN_PASSWORD),
            ],
        )
        .await;
    assert!(result.body.contains("Couldn't save your changes."));
    let unchanged = owner(&site).await;
    assert_eq!(unchanged.email, me.email);
    assert_eq!(unchanged.name, me.name);
    assert_eq!(unchanged.session_version, me.session_version);
    assert!(has_link(&site, &token).await);
    assert_eq!(site.get("/admin").await.status, StatusCode::OK);
}

#[tokio::test]
async fn reset_link_replacement_is_atomic_and_rolls_back_on_insert_failure() {
    let site = TestSite::with_owner().await;
    let me = owner(&site).await;
    let (first, second) = tokio::join!(
        create_link_if_current(&site.pool, me.id, me.session_version),
        create_link_if_current(&site.pool, me.id, me.session_version)
    );
    let first = first.unwrap().unwrap();
    let second = second.unwrap().unwrap();
    assert_ne!(
        has_link(&site, &first).await,
        has_link(&site, &second).await
    );
    let survivor = if has_link(&site, &first).await {
        first
    } else {
        second
    };
    sqlx::query("CREATE TRIGGER fail_reset_insert BEFORE INSERT ON password_resets BEGIN SELECT RAISE(ABORT, 'test failure'); END")
        .execute(&site.pool).await.unwrap();
    assert!(create_link(&site.pool, me.id).await.is_err());
    assert!(has_link(&site, &survivor).await);
}

#[tokio::test]
async fn a_stolen_cookie_and_setup_code_cannot_enable_two_factor() {
    let mut site = TestSite::with_owner().await;
    let me = owner(&site).await;
    let original_cookie = site.cookie.clone();
    site.new_visitor();
    assert_eq!(
        site.sign_in(ADMIN_EMAIL, ADMIN_PASSWORD).await.location(),
        "/admin"
    );
    let secret = setup_key(&mut site).await;
    for password in [None, Some("wrong")] {
        let code = authenticator_code(&secret);
        let mut fields = vec![("code", code.as_str())];
        if let Some(password) = password {
            fields.push(("password", password));
        }
        let result = site.post("/admin/profile/two-factor", &fields).await;
        assert_eq!(result.status, StatusCode::UNPROCESSABLE_ENTITY);
        assert!(result.body.contains("Your current password isn"));
        assert!(
            result.body.contains("two-factor-key"),
            "allow a retry with the same setup key"
        );
        assert!(users::two_factor(&site.pool, me.id).await.is_none());
        assert_eq!(users::recovery_codes_left(&site.pool, me.id).await, 0);
        assert_eq!(owner(&site).await.session_version, me.session_version);
    }
    let setup_cookie = site.cookie.clone();
    site.cookie = original_cookie;
    assert_eq!(site.get("/admin").await.status, StatusCode::OK);
    site.cookie = setup_cookie;
    let result = site
        .post(
            "/admin/profile/two-factor",
            &[
                ("code", &authenticator_code(&secret)),
                ("password", ADMIN_PASSWORD),
            ],
        )
        .await;
    assert_eq!(result.status, StatusCode::OK);
    assert!(result.body.contains("<li><code>"));
    assert!(users::two_factor(&site.pool, me.id).await.is_some());
    assert_eq!(users::recovery_codes_left(&site.pool, me.id).await, 10);
}

#[tokio::test]
async fn all_sensitive_profile_forms_share_the_login_attempt_budget() {
    let mut site = TestSite::with_owner().await;
    let me = owner(&site).await;
    let secret = setup_key(&mut site).await;
    // Keep an authenticated session while enabling a factor for the recovery form.
    sqlx::query("UPDATE users SET totp_secret = ? WHERE id = ?")
        .bind(crate::app::security::to_hex(&secret))
        .bind(me.id)
        .execute(&site.pool)
        .await
        .unwrap();
    let cookie = site.cookie.clone();
    site.new_visitor();
    for _ in 0..5 {
        site.sign_in(ADMIN_EMAIL, "wrong").await;
    }
    site.cookie = cookie;
    let cases = [
        (
            "/admin/profile/password",
            vec![
                ("current_password", ADMIN_PASSWORD),
                ("new_password", "Replacement-Password-7"),
                ("confirm_password", "Replacement-Password-7"),
            ],
        ),
        (
            "/admin/profile/two-factor/off",
            vec![("password", ADMIN_PASSWORD)],
        ),
        (
            "/admin/profile/two-factor/recovery",
            vec![("password", ADMIN_PASSWORD)],
        ),
        (
            "/admin/profile",
            vec![
                ("name", "Owner"),
                ("email", "attacker@example.com"),
                ("current_password", ADMIN_PASSWORD),
            ],
        ),
    ];
    for (path, fields) in cases {
        let result = site.post(path, &fields).await;
        assert_eq!(result.status, StatusCode::TOO_MANY_REQUESTS, "{path}");
        assert!(result.header("content-type").starts_with("text/html"));
        assert!(result.body.contains("Too many failed sign-ins"));
    }
    let result = site
        .post(
            "/admin/profile/two-factor",
            &[
                ("password", ADMIN_PASSWORD),
                ("code", &authenticator_code(&secret)),
            ],
        )
        .await;
    assert_eq!(result.status, StatusCode::TOO_MANY_REQUESTS);
    assert_eq!(owner(&site).await.session_version, me.session_version);
    assert_eq!(owner(&site).await.email, ADMIN_EMAIL);
    assert!(users::two_factor(&site.pool, me.id).await.is_some());
    assert_eq!(users::recovery_codes_left(&site.pool, me.id).await, 0);
}

#[tokio::test]
async fn parallel_guesses_across_profile_forms_are_limited_to_five_checks() {
    let mut site = TestSite::with_owner().await;
    let me = owner(&site).await;
    let secret = setup_key(&mut site).await;
    sqlx::query("UPDATE users SET totp_secret = ? WHERE id = ?")
        .bind(crate::app::security::to_hex(&secret))
        .bind(me.id)
        .execute(&site.pool)
        .await
        .unwrap();
    let cases = [
        ("/admin/profile/password", "current_password=wrong&new_password=Replacement-Password-7&confirm_password=Replacement-Password-7"),
        ("/admin/profile/two-factor/off", "password=wrong"),
        ("/admin/profile/two-factor/recovery", "password=wrong"),
        ("/admin/profile", "name=Owner&email=attacker%40example.com&current_password=wrong"),
        ("/admin/profile/two-factor", "password=wrong&code=000000"),
    ];
    let requests = (0..20).map(|n| {
        let (path, fields) = cases[n % cases.len()];
        let mut request = Request::post(path)
            .header(header::CONTENT_TYPE, "application/x-www-form-urlencoded")
            .header("sec-fetch-site", "same-origin")
            .header(header::COOKIE, site.cookie.as_ref().unwrap())
            .body(Body::from(fields))
            .unwrap();
        request.extensions_mut().insert(ConnectInfo(site.address));
        site.app.clone().oneshot(request)
    });
    let responses = futures_util::future::join_all(requests).await;
    assert_eq!(
        responses
            .iter()
            .filter(|r| r.as_ref().unwrap().status() == StatusCode::UNPROCESSABLE_ENTITY)
            .count(),
        5
    );
    assert_eq!(
        responses
            .iter()
            .filter(|r| r.as_ref().unwrap().status() == StatusCode::TOO_MANY_REQUESTS)
            .count(),
        15
    );
    assert_eq!(owner(&site).await.email, ADMIN_EMAIL);
    assert_eq!(owner(&site).await.session_version, me.session_version);
    assert!(users::two_factor(&site.pool, me.id).await.is_some());
}

#[tokio::test]
async fn a_successful_confirmation_does_not_clear_other_failed_guesses() {
    let mut site = TestSite::with_owner().await;
    let secret = setup_key(&mut site).await;
    for _ in 0..4 {
        assert_eq!(
            site.post("/admin/profile/two-factor/off", &[("password", "wrong")])
                .await
                .status,
            StatusCode::UNPROCESSABLE_ENTITY
        );
    }
    let now = chrono::Utc::now().timestamp() as u64;
    let wrong = (0..10)
        .map(|n| format!("{n:06}"))
        .find(|code| crate::app::totp::verify(&secret, code, now, 0).is_none())
        .unwrap();
    let result = site
        .post(
            "/admin/profile/two-factor",
            &[("password", ADMIN_PASSWORD), ("code", &wrong)],
        )
        .await;
    assert!(result.body.contains("That code isn"));
    assert_eq!(
        site.post("/admin/profile/two-factor/off", &[("password", "wrong")])
            .await
            .status,
        StatusCode::UNPROCESSABLE_ENTITY
    );
    assert_eq!(
        site.post("/admin/profile/two-factor/off", &[("password", "wrong")])
            .await
            .status,
        StatusCode::TOO_MANY_REQUESTS
    );
}
