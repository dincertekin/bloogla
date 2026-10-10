//! Account revocation, parallel guessing and private database copies.

use super::{TestSite, ADMIN_EMAIL, ADMIN_PASSWORD};
use axum::body::Body;
use axum::extract::ConnectInfo;
use axum::http::{header, Request, StatusCode};
use tower::ServiceExt;

async fn owner_id(site: &TestSite) -> i64 {
    sqlx::query_scalar("SELECT id FROM users WHERE email = ?")
        .bind(ADMIN_EMAIL)
        .fetch_one(&site.pool)
        .await
        .unwrap()
}

async fn reset_link_exists(site: &TestSite, token: &str) -> bool {
    sqlx::query_scalar::<_, i64>("SELECT COUNT(*) FROM password_resets WHERE token_hash = ?")
        .bind(crate::app::security::hash_token(token))
        .fetch_one(&site.pool)
        .await
        .unwrap()
        == 1
}

#[tokio::test]
async fn concurrent_guesses_respect_the_account_budget_for_both_login_steps() {
    for two_factor in [false, true] {
        let mut site = TestSite::with_owner().await;
        // Separate accounts keep this test's deliberate failures out of other tests.
        let email = format!("burst-{}@example.com", crate::app::security::random_hex(8));
        let secret = b"12345678901234567890";
        sqlx::query("UPDATE users SET email = ?, totp_secret = ?")
            .bind(&email)
            .bind(two_factor.then(|| crate::app::security::to_hex(secret)))
            .execute(&site.pool)
            .await
            .unwrap();
        site.new_visitor();
        if two_factor {
            assert_eq!(
                site.sign_in(&email, ADMIN_PASSWORD).await.location(),
                "/admin/login/code"
            );
        }
        let now = chrono::Utc::now().timestamp() as u64;
        let wrong_code = (0..10)
            .map(|n| format!("{n:06}"))
            .find(|code| crate::app::totp::verify(secret, code, now, 0).is_none())
            .unwrap();
        let requests = (0..20).map(|n| {
            let (path, fields) = if two_factor {
                ("/admin/login/code", format!("code={wrong_code}"))
            } else {
                ("/admin/login", format!("email={email}&password=wrong"))
            };
            let mut request = Request::post(path)
                .header(header::CONTENT_TYPE, "application/x-www-form-urlencoded")
                .header("sec-fetch-site", "same-origin");
            if let Some(cookie) = &site.cookie {
                request = request.header(header::COOKIE, cookie);
            }
            let mut request = request.body(Body::from(fields)).unwrap();
            // Guesses spread across addresses must still share one budget.
            request
                .extensions_mut()
                .insert(ConnectInfo(std::net::SocketAddr::from((
                    [192, 0, 2, n],
                    50000,
                ))));
            site.app.clone().oneshot(request)
        });
        let responses = futures_util::future::join_all(requests).await;
        let checked = responses
            .iter()
            .filter(|r| r.as_ref().unwrap().status() == StatusCode::OK)
            .count();
        assert_eq!(checked, 5, "two_factor={two_factor}");
        assert_eq!(
            responses
                .iter()
                .filter(|r| r.as_ref().unwrap().status() == StatusCode::TOO_MANY_REQUESTS)
                .count(),
            15
        );
        // Knowing the password must not reset the failed-code budget.
        if two_factor {
            assert_eq!(
                site.sign_in(&email, ADMIN_PASSWORD).await.status,
                StatusCode::TOO_MANY_REQUESTS
            );
        }
    }
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

#[tokio::test]
async fn a_stale_password_change_cannot_replace_the_password_or_restore_its_session() {
    let mut site = TestSite::with_owner().await;
    let new_password = "Owner-New-Password-8";
    let hash = crate::app::security::hash_password(new_password).unwrap();
    let mut tx = site.pool.begin_with("BEGIN IMMEDIATE").await.unwrap();
    sqlx::query("UPDATE users SET password_hash = ?, session_version = session_version + 1")
        .bind(&hash)
        .execute(&mut *tx)
        .await
        .unwrap();
    // WAL readers can still check the old password while the owner holds
    // this write lock. The conditional update must reject that stale result.
    let request = site.post(
        "/admin/profile/password",
        &[
            ("current_password", ADMIN_PASSWORD),
            ("new_password", "Stale-Request-Password-9"),
            ("confirm_password", "Stale-Request-Password-9"),
        ],
    );
    let (result, _) = tokio::join!(request, async move {
        tokio::time::sleep(std::time::Duration::from_millis(300)).await;
        tx.commit().await.unwrap();
    });
    assert!(!result.body.contains("Password updated."));
    assert_eq!(site.get("/admin").await.location(), "/admin/login");
    let (stored, version): (String, i64) =
        sqlx::query_as("SELECT password_hash, session_version FROM users")
            .fetch_one(&site.pool)
            .await
            .unwrap();
    assert_eq!(stored, hash);
    assert_eq!(version, 1);
    assert_eq!(
        site.sign_in(ADMIN_EMAIL, new_password).await.location(),
        "/admin"
    );
}

#[tokio::test]
async fn stale_password_updates_preserve_newer_reset_links() {
    let site = TestSite::with_owner().await;
    let id = owner_id(&site).await;
    let hash = crate::app::security::hash_password("Changed-Good-Password-8").unwrap();
    crate::db::users::set_password(&site.pool, id, &hash)
        .await
        .unwrap();
    let token = crate::handlers::admin::password_reset::create_link(&site.pool, id)
        .await
        .unwrap();
    assert_eq!(
        crate::db::users::change_password(&site.pool, id, "stale hash", Some(0))
            .await
            .unwrap(),
        None
    );
    assert!(reset_link_exists(&site, &token).await);
}

#[tokio::test]
async fn all_password_change_paths_revoke_older_reset_links() {
    for path in ["profile", "cli"] {
        let mut site = TestSite::with_owner().await;
        let id = owner_id(&site).await;
        let token = crate::handlers::admin::password_reset::create_link(&site.pool, id)
            .await
            .unwrap();
        let password = "Changed-Good-Password-8";
        match path {
            "profile" => {
                let result = site
                    .post(
                        "/admin/profile/password",
                        &[
                            ("current_password", ADMIN_PASSWORD),
                            ("new_password", password),
                            ("confirm_password", password),
                        ],
                    )
                    .await;
                assert!(result.body.contains("Password updated."));
            }
            _ => {
                // The CLI uses this same database operation after prompting.
                let hash = crate::app::security::hash_password(password).unwrap();
                crate::db::users::set_password(&site.pool, id, &hash)
                    .await
                    .unwrap();
            }
        }
        assert!(!reset_link_exists(&site, &token).await, "{path}");
        site.new_visitor();
        let result = site
            .post(
                "/admin/reset-password",
                &[
                    ("token", &token),
                    ("password", "Attacker-Good-Password-9"),
                    ("confirm_password", "Attacker-Good-Password-9"),
                ],
            )
            .await;
        assert_ne!(result.location(), "/admin/login?reset=1", "{path}");
        assert_eq!(
            site.sign_in(ADMIN_EMAIL, password).await.location(),
            "/admin",
            "{path}"
        );
    }
}

#[tokio::test]
async fn an_administrator_password_reset_revokes_the_old_link() {
    let mut site = TestSite::with_owner().await;
    let hash = crate::app::security::hash_password(ADMIN_PASSWORD).unwrap();
    let id: i64 = sqlx::query_scalar(
        "INSERT INTO users (name, email, password_hash, role)
        VALUES ('Author', 'author@example.com', ?, 'author') RETURNING id",
    )
    .bind(hash)
    .fetch_one(&site.pool)
    .await
    .unwrap();
    let token = crate::handlers::admin::password_reset::create_link(&site.pool, id)
        .await
        .unwrap();
    let result = site.post(&format!("/admin/users/{id}/password"), &[]).await;
    assert_eq!(result.status, StatusCode::OK);
    let password = result
        .body
        .split("<code>")
        .nth(1)
        .unwrap()
        .split("</code>")
        .next()
        .unwrap();
    assert!(!reset_link_exists(&site, &token).await);
    site.new_visitor();
    assert_eq!(
        site.sign_in("author@example.com", password)
            .await
            .location(),
        "/admin"
    );
}

pub(super) fn authenticator_code(secret: &[u8]) -> String {
    use hmac::{Hmac, Mac};
    let step = chrono::Utc::now().timestamp() as u64 / 30;
    let mut mac = Hmac::<sha1::Sha1>::new_from_slice(secret).unwrap();
    mac.update(&step.to_be_bytes());
    let digest = mac.finalize().into_bytes();
    let offset = (digest[19] & 15) as usize;
    let value = u32::from_be_bytes(digest[offset..offset + 4].try_into().unwrap()) & 0x7fff_ffff;
    format!("{:06}", value % 1_000_000)
}

#[tokio::test]
async fn enabling_two_factor_revokes_other_devices_and_keeps_the_confirming_device() {
    let mut site = TestSite::with_owner().await;
    let other_cookie = site.cookie.clone();
    site.new_visitor();
    assert_eq!(
        site.sign_in(ADMIN_EMAIL, ADMIN_PASSWORD).await.location(),
        "/admin"
    );
    let page = site.get("/admin/profile/two-factor").await;
    let displayed = page
        .body
        .split("<code class=\"two-factor-key\">")
        .nth(1)
        .unwrap()
        .split("</code>")
        .next()
        .unwrap();
    // Decode the setup key as an authenticator would, without accessing
    // session internals or using Bloogla's code generation implementation.
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
    let recovery = result
        .body
        .split("<li><code>")
        .nth(1)
        .unwrap()
        .split("</code>")
        .next()
        .unwrap();
    assert_eq!(site.get("/admin").await.status, StatusCode::OK);
    site.cookie = other_cookie;
    assert_eq!(site.get("/admin").await.location(), "/admin/login");
    assert_eq!(
        site.post(
            "/admin/profile/two-factor/recovery",
            &[("password", ADMIN_PASSWORD)]
        )
        .await
        .status,
        StatusCode::UNAUTHORIZED
    );
    site.new_visitor();
    assert_eq!(
        site.sign_in(ADMIN_EMAIL, ADMIN_PASSWORD).await.location(),
        "/admin/login/code"
    );
    assert_eq!(
        site.post("/admin/login/code", &[("code", recovery)])
            .await
            .location(),
        "/admin"
    );
    assert_eq!(site.get("/admin").await.status, StatusCode::OK);
}

#[tokio::test]
async fn password_changes_roll_back_when_reset_link_revocation_fails() {
    let site = TestSite::with_owner().await;
    let id = owner_id(&site).await;
    let token = crate::handlers::admin::password_reset::create_link(&site.pool, id)
        .await
        .unwrap();
    sqlx::query(
        "CREATE TRIGGER refuse_reset_revocation BEFORE DELETE ON password_resets
        BEGIN SELECT RAISE(ABORT, 'test write failure'); END",
    )
    .execute(&site.pool)
    .await
    .unwrap();
    assert!(crate::db::users::set_password(&site.pool, id, "new hash")
        .await
        .is_err());
    let (hash, version): (String, i64) =
        sqlx::query_as("SELECT password_hash, session_version FROM users")
            .fetch_one(&site.pool)
            .await
            .unwrap();
    assert!(crate::app::security::verify_password(
        ADMIN_PASSWORD,
        Some(&hash)
    ));
    assert_eq!(version, 0);
    assert!(reset_link_exists(&site, &token).await);
}

#[tokio::test]
async fn recovery_code_replacement_requires_the_current_password() {
    let mut site = TestSite::with_owner().await;
    let id = owner_id(&site).await;
    let old_code = "abcde-fghjk";
    sqlx::query("UPDATE users SET totp_secret = ?")
        .bind(crate::app::security::to_hex(b"12345678901234567890"))
        .execute(&site.pool)
        .await
        .unwrap();
    sqlx::query("INSERT INTO recovery_codes (user_id, code_hash) VALUES (?, ?)")
        .bind(id)
        .bind(crate::app::security::hash_recovery_code(old_code))
        .execute(&site.pool)
        .await
        .unwrap();
    let old_cookie = site.cookie.clone();
    for fields in [vec![], vec![("password", "wrong")]] {
        let result = site
            .post("/admin/profile/two-factor/recovery", &fields)
            .await;
        assert_eq!(
            result.status,
            if fields.is_empty() {
                StatusCode::BAD_REQUEST
            } else {
                StatusCode::UNPROCESSABLE_ENTITY
            }
        );
        assert!(!result.body.contains("<li><code>"));
        assert_eq!(
            crate::db::users::recovery_codes_left(&site.pool, id).await,
            1
        );
    }
    let result = site
        .post(
            "/admin/profile/two-factor/recovery",
            &[("password", ADMIN_PASSWORD)],
        )
        .await;
    assert_eq!(result.status, StatusCode::OK);
    assert!(result.body.contains("<li><code>"));
    assert_eq!(
        crate::db::users::recovery_codes_left(&site.pool, id).await,
        10
    );
    assert!(!crate::db::users::use_recovery_code(&site.pool, id, old_code).await);
    assert_eq!(site.get("/admin").await.status, StatusCode::OK);
    site.cookie = old_cookie;
    assert_eq!(site.get("/admin").await.location(), "/admin/login");
}

#[tokio::test]
async fn stale_two_factor_changes_cannot_reissue_credentials() {
    let site = TestSite::with_owner().await;
    let id = owner_id(&site).await;
    let secret = b"12345678901234567890";
    let hashes = vec![crate::app::security::hash_recovery_code("abcde-fghjk")];
    assert_eq!(
        crate::db::users::enable_two_factor(&site.pool, id, secret, 1, &hashes, 0)
            .await
            .unwrap(),
        Some(1)
    );
    assert_eq!(
        crate::db::users::enable_two_factor(&site.pool, id, b"another secret", 2, &[], 0)
            .await
            .unwrap(),
        None
    );
    assert_eq!(
        crate::db::users::set_recovery_codes(&site.pool, id, &[], 0)
            .await
            .unwrap(),
        None
    );
    assert!(
        !crate::db::users::disable_two_factor_if_current(&site.pool, id, Some(0))
            .await
            .unwrap()
    );
    assert_eq!(
        crate::db::users::recovery_codes_left(&site.pool, id).await,
        1
    );
    assert_eq!(
        crate::db::users::two_factor(&site.pool, id)
            .await
            .unwrap()
            .0,
        secret
    );
}

#[cfg(unix)]
#[tokio::test]
async fn databases_and_backup_archives_are_private_including_existing_files() {
    use std::os::unix::fs::PermissionsExt;
    let site = TestSite::with_owner().await;
    let mode =
        |path: &std::path::Path| std::fs::metadata(path).unwrap().permissions().mode() & 0o777;
    for suffix in ["", "-wal", "-shm"] {
        assert_eq!(
            mode(std::path::Path::new(&format!(
                "{}{suffix}",
                site.database.display()
            ))),
            0o600
        );
    }
    let folder = std::env::temp_dir().join(format!(
        "bloogla-private-test-{}",
        crate::app::security::random_hex(8)
    ));
    let backup = folder.join("backup.db");
    crate::services::backup::backup_to(&site.pool, &backup)
        .await
        .unwrap();
    assert_eq!(mode(&folder), 0o700);
    assert_eq!(mode(&backup), 0o600);
    // Opening an old, permissively-created database tightens it as well.
    std::fs::set_permissions(&backup, std::fs::Permissions::from_mode(0o644)).unwrap();
    let pool = crate::db::connect(&format!("sqlite://{}?mode=rwc", backup.display()))
        .await
        .unwrap();
    assert_eq!(mode(&backup), 0o600);
    pool.close().await;
    let original = std::fs::read(&backup).unwrap();
    assert!(crate::services::backup::backup_to(&site.pool, &backup)
        .await
        .is_err());
    assert_eq!(
        std::fs::read(&backup).unwrap(),
        original,
        "backups cannot overwrite existing files"
    );
    let archive = crate::services::backup::download_archive(&site.pool, &folder.join("uploads"))
        .await
        .unwrap();
    assert_eq!(mode(&archive), 0o600);
    assert_eq!(
        mode(std::path::Path::new(crate::services::backup::BACKUP_DIR)),
        0o700
    );
    let mut zip = zip::ZipArchive::new(std::fs::File::open(&archive).unwrap()).unwrap();
    assert_eq!(
        zip.by_name("data/bloogla.db").unwrap().unix_mode().unwrap() & 0o777,
        0o600
    );
    drop(zip);
    std::fs::remove_file(archive).unwrap();
    std::fs::remove_dir_all(folder).unwrap();
}
