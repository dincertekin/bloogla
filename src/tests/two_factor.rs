//! Two-factor login: password changes revoke pending sign-ins and codes are
//! only accepted when their use has been recorded successfully.

use super::{TestSite, ADMIN_EMAIL, ADMIN_PASSWORD};
use axum::body::Body;
use axum::extract::ConnectInfo;
use axum::http::{header, Request, StatusCode};
use hmac::{Hmac, Mac};
use sha1::Sha1;
use tower::ServiceExt;

const SECRET: &[u8] = b"12345678901234567890";
const RECOVERY_CODE: &str = "abcde-fghjk";

async fn enable_two_factor(site: &TestSite) -> i64 {
    let id: i64 = sqlx::query_scalar("SELECT id FROM users WHERE email = ?")
        .bind(ADMIN_EMAIL)
        .fetch_one(&site.pool)
        .await
        .unwrap();
    // Legacy plaintext keys are supported; this fixture avoids changing the
    // process-wide encryption key used by the encryption tests.
    sqlx::query("UPDATE users SET totp_secret = ?, totp_last_step = 0 WHERE id = ?")
        .bind(crate::app::security::to_hex(SECRET))
        .bind(id)
        .execute(&site.pool)
        .await
        .unwrap();
    id
}

async fn pending_cookie(site: &mut TestSite) -> String {
    site.new_visitor();
    assert_eq!(
        site.sign_in(ADMIN_EMAIL, ADMIN_PASSWORD).await.location(),
        "/admin/login/code"
    );
    site.cookie.clone().unwrap()
}

/// Generate the code a reader's authenticator would show, independently of
/// Bloogla's verification function.
fn authenticator_code() -> String {
    let step = chrono::Utc::now().timestamp() as u64 / 30;
    let mut mac = Hmac::<Sha1>::new_from_slice(SECRET).unwrap();
    mac.update(&step.to_be_bytes());
    let digest = mac.finalize().into_bytes();
    let offset = (digest[19] & 15) as usize;
    let value = u32::from_be_bytes(digest[offset..offset + 4].try_into().unwrap()) & 0x7fff_ffff;
    format!("{:06}", value % 1_000_000)
}

fn code_request(site: &TestSite, cookie: &str, code: &str) -> Request<Body> {
    let mut request = Request::post("/admin/login/code")
        .header(header::CONTENT_TYPE, "application/x-www-form-urlencoded")
        .header("sec-fetch-site", "same-origin")
        .header(header::COOKIE, cookie)
        .body(Body::from(format!("code={code}")))
        .unwrap();
    request.extensions_mut().insert(ConnectInfo(site.address));
    request
}

#[tokio::test]
async fn a_password_change_revokes_pending_two_factor_logins() {
    let mut site = TestSite::with_owner().await;
    let id = enable_two_factor(&site).await;
    sqlx::query("INSERT INTO recovery_codes (user_id, code_hash) VALUES (?, ?)")
        .bind(id)
        .bind(crate::app::security::hash_recovery_code(RECOVERY_CODE))
        .execute(&site.pool)
        .await
        .unwrap();
    let totp_cookie = pending_cookie(&mut site).await;
    let recovery_cookie = pending_cookie(&mut site).await;
    let page_cookie = pending_cookie(&mut site).await;

    let password = "Changed-Good-Password-8";
    let hash = crate::app::security::hash_password(password).unwrap();
    crate::db::users::set_password(&site.pool, id, &hash)
        .await
        .unwrap();

    // Check both POST paths directly; neither may consume a code or sign in.
    for (cookie, code) in [
        (totp_cookie, authenticator_code()),
        (recovery_cookie, RECOVERY_CODE.to_string()),
    ] {
        site.cookie = Some(cookie);
        assert_eq!(
            site.post("/admin/login/code", &[("code", &code)])
                .await
                .location(),
            "/admin/login"
        );
        assert_eq!(site.get("/admin").await.location(), "/admin/login");
    }
    site.cookie = Some(page_cookie);
    assert_eq!(
        site.get("/admin/login/code").await.location(),
        "/admin/login"
    );
    assert_eq!(
        crate::db::users::recovery_codes_left(&site.pool, id).await,
        1
    );

    // The same recovery code still works after checking the new password.
    site.new_visitor();
    assert_eq!(
        site.sign_in(ADMIN_EMAIL, password).await.location(),
        "/admin/login/code"
    );
    assert_eq!(
        site.post("/admin/login/code", &[("code", RECOVERY_CODE)])
            .await
            .location(),
        "/admin"
    );
    assert_eq!(site.get("/admin").await.status, StatusCode::OK);
}

#[tokio::test]
async fn an_authenticator_code_signs_in_only_one_concurrent_visitor() {
    let mut site = TestSite::with_owner().await;
    enable_two_factor(&site).await;
    let cookies = [
        pending_cookie(&mut site).await,
        pending_cookie(&mut site).await,
    ];
    let code = authenticator_code();
    let (first, second) = tokio::join!(
        site.app
            .clone()
            .oneshot(code_request(&site, &cookies[0], &code)),
        site.app
            .clone()
            .oneshot(code_request(&site, &cookies[1], &code)),
    );
    let responses = [first.unwrap(), second.unwrap()];
    assert_eq!(
        responses
            .iter()
            .filter(|r| r.status() == StatusCode::SEE_OTHER)
            .count(),
        1,
        "only the visitor who recorded the code's use may sign in"
    );
    let winner = responses
        .iter()
        .position(|r| r.status() == StatusCode::SEE_OTHER)
        .unwrap();
    site.cookie = Some(
        responses[winner].headers()[header::SET_COOKIE]
            .to_str()
            .unwrap()
            .split(';')
            .next()
            .unwrap()
            .to_string(),
    );
    assert_eq!(site.get("/admin").await.status, StatusCode::OK);
    for response in responses {
        if response.status() == StatusCode::SEE_OTHER {
            assert_eq!(response.headers()[header::LOCATION], "/admin");
        } else {
            assert_eq!(response.status(), StatusCode::OK);
            let bytes = axum::body::to_bytes(response.into_body(), usize::MAX)
                .await
                .unwrap();
            assert!(String::from_utf8_lossy(&bytes).contains("Try the newest one from your app."));
        }
    }
    // Replaying the code after both requests have finished also fails.
    site.cookie = Some(cookies[1 - winner].clone());
    assert_eq!(
        site.post("/admin/login/code", &[("code", &code)])
            .await
            .status,
        StatusCode::OK
    );
    assert_eq!(site.get("/admin").await.location(), "/admin/login");
}

#[tokio::test]
async fn a_code_step_can_be_claimed_only_once_even_with_stale_reads() {
    let site = TestSite::with_owner().await;
    let id = enable_two_factor(&site).await;
    // Both verifications saw the same unused step. The database claim must
    // still refuse one of them, regardless of which request runs first.
    let (first, second) = tokio::join!(
        crate::db::users::set_two_factor_step(&site.pool, id, 100),
        crate::db::users::set_two_factor_step(&site.pool, id, 100),
    );
    assert_ne!(first, second);
    assert!(!crate::db::users::set_two_factor_step(&site.pool, id, 99).await);
}

#[tokio::test]
async fn a_failed_code_write_does_not_sign_the_visitor_in() {
    let mut site = TestSite::with_owner().await;
    enable_two_factor(&site).await;
    pending_cookie(&mut site).await;
    sqlx::query(
        "CREATE TRIGGER refuse_code_use BEFORE UPDATE OF totp_last_step ON users
         BEGIN SELECT RAISE(ABORT, 'test write failure'); END",
    )
    .execute(&site.pool)
    .await
    .unwrap();
    let page = site
        .post("/admin/login/code", &[("code", &authenticator_code())])
        .await;
    assert_eq!(page.status, StatusCode::OK);
    assert!(page.body.contains("Try the newest one from your app."));
    assert_eq!(site.get("/admin").await.location(), "/admin/login");
}
