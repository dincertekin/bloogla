//! Email subscriptions: the form under posts, confirming, unsubscribing,
//! the editor's checkbox and which posts go out.

use super::TestSite;
use axum::http::StatusCode;

/// The form's hidden opening time, `seconds_ago`.
fn opened(seconds_ago: i64) -> String {
    let when = chrono::Utc::now().timestamp() - seconds_ago;
    crate::app::secrets::encrypt(&when.to_string())
}

/// A site with email set up and subscriptions on, and one published post.
async fn site_with_subscriptions() -> TestSite {
    let mut site = TestSite::with_owner().await;
    crate::db::settings::save(
        &site.pool,
        &[
            ("smtp_host", "mail.invalid".into()),
            ("smtp_from", "blog@example.com".into()),
        ],
    )
    .await
    .unwrap();
    let saved = site
        .post("/admin/settings/subscribers", &[("subscriptions_on", "1")])
        .await;
    assert!(saved.body.contains("Saved."), "{}", saved.body);
    site.publish("First Post", "published").await;
    site
}

async fn subscriber(site: &TestSite, email: &str) -> Option<(String, bool)> {
    sqlx::query_as("SELECT token, confirmed_at IS NOT NULL FROM subscribers WHERE email = ?")
        .bind(email)
        .fetch_optional(&site.pool)
        .await
        .unwrap()
}

#[tokio::test]
async fn subscriptions_are_off_until_turned_on_and_need_email() {
    let mut site = TestSite::with_owner().await;
    let url = site.publish("Quiet Post", "published").await;
    assert!(!site.get(&url).await.body.contains("subscribe-form"));
    let refused = site
        .post(
            &format!("{url}/subscribe"),
            &[("email", "reader@example.com")],
        )
        .await;
    assert_eq!(refused.status, StatusCode::NOT_FOUND);

    // Email isn't set up, so it can't be turned on.
    let page = site
        .post("/admin/settings/subscribers", &[("subscriptions_on", "1")])
        .await;
    assert_eq!(page.status, StatusCode::UNPROCESSABLE_ENTITY);
    assert!(!crate::db::settings::load(&site.pool).await.subscriptions_on);
}

#[tokio::test]
async fn a_reader_subscribes_confirms_and_unsubscribes() {
    let mut site = site_with_subscriptions().await;
    site.new_visitor();
    let post = site.get("/post/first-post").await.body;
    assert!(post.contains(r#"action="/post/first-post/subscribe""#));

    let in_time = opened(10);
    let sent = site
        .post(
            "/post/first-post/subscribe",
            &[("email", "reader@example.com"), ("opened", &in_time)],
        )
        .await;
    assert_eq!(
        sent.location(),
        "/post/first-post?subscribe=check#subscribe"
    );
    let (token, confirmed) = subscriber(&site, "reader@example.com").await.unwrap();
    assert!(!confirmed, "waits for the link in the email");
    assert!(site
        .get(sent.location())
        .await
        .body
        .contains("open the email we just sent you"));

    // Asking again keeps the same link.
    site.post(
        "/post/first-post/subscribe",
        &[("email", "Reader@Example.com"), ("opened", &in_time)],
    )
    .await;
    assert_eq!(
        subscriber(&site, "reader@example.com").await.unwrap().0,
        token
    );

    let confirmed = site.get(&format!("/subscribe/{token}")).await;
    assert_eq!(confirmed.status, StatusCode::OK);
    assert!(confirmed.body.contains("subscribed"));
    assert_eq!(confirmed.header("cache-control"), "no-store");
    assert!(subscriber(&site, "reader@example.com").await.unwrap().1);
    assert_eq!(
        site.get("/subscribe/not-a-token").await.status,
        StatusCode::NOT_FOUND
    );

    // Opening the unsubscribe link only asks; the button unsubscribes.
    let asked = site.get(&format!("/unsubscribe/{token}")).await;
    assert!(asked
        .body
        .contains(&format!(r#"action="/unsubscribe/{token}""#)));
    assert!(subscriber(&site, "reader@example.com").await.is_some());
    // Email apps' own button posts without a browser's headers.
    let request = axum::http::Request::post(format!("/unsubscribe/{token}"));
    let done = site.send(request, axum::body::Body::empty()).await;
    assert_eq!(done.status, StatusCode::OK);
    assert!(subscriber(&site, "reader@example.com").await.is_none());
}

#[tokio::test]
async fn bots_and_bad_addresses_are_not_added() {
    let mut site = site_with_subscriptions().await;
    site.new_visitor();
    let quick = opened(0);
    let in_time = opened(10);
    for fields in [
        vec![("email", "bot@example.com"), ("opened", quick.as_str())],
        vec![
            ("email", "bot@example.com"),
            ("opened", in_time.as_str()),
            ("website", "spam.example"),
        ],
    ] {
        let page = site.post("/post/first-post/subscribe", &fields).await;
        assert!(
            page.location().contains("subscribe=check"),
            "looks fine to bots"
        );
    }
    assert!(subscriber(&site, "bot@example.com").await.is_none());

    let bad = site
        .post(
            "/post/first-post/subscribe",
            &[("email", "not an address"), ("opened", &in_time)],
        )
        .await;
    assert!(bad.location().contains("subscribe=invalid"));
}

#[tokio::test]
async fn a_ticked_post_is_sent_once_and_old_posts_never() {
    let mut site = site_with_subscriptions().await;

    // The editor offers the box on new posts, ticked.
    let editor = site.get("/admin/posts/new").await.body;
    assert!(editor.contains(r#"name="email_subscribers" value="1" checked"#));
    site.post(
        "/admin/posts",
        &[
            ("title", "Big News"),
            ("content", "Text"),
            ("status", "draft"),
            ("email_choice", "1"),
            ("email_subscribers", "1"),
        ],
    )
    .await;
    let id: i64 = sqlx::query_scalar("SELECT id FROM posts WHERE slug = 'big-news'")
        .fetch_one(&site.pool)
        .await
        .unwrap();
    // A draft isn't sent.
    assert!(crate::db::posts::due_for_subscribers(&site.pool)
        .await
        .is_empty());

    site.post(
        &format!("/admin/posts/{id}/edit"),
        &[
            ("title", "Big News"),
            ("content", "Text"),
            ("status", "published"),
            ("email_choice", "1"),
            ("email_subscribers", "1"),
        ],
    )
    .await;
    let due = crate::db::posts::due_for_subscribers(&site.pool).await;
    assert_eq!(due.len(), 1);
    assert!(crate::db::posts::mark_emailed(&site.pool, id).await);
    assert!(
        !crate::db::posts::mark_emailed(&site.pool, id).await,
        "only once"
    );
    assert!(crate::db::posts::due_for_subscribers(&site.pool)
        .await
        .is_empty());
    // Once sent (or public), the editor doesn't offer it again.
    let editor = site.get(&format!("/admin/posts/{id}/edit")).await.body;
    assert!(!editor.contains(r#"name="email_subscribers""#));

    // A post published long ago is never sent, even if ticked.
    sqlx::query(
        "UPDATE posts SET email_subscribers = 1, published_at = datetime('now', '-3 days')
         WHERE slug = 'first-post'",
    )
    .execute(&site.pool)
    .await
    .unwrap();
    assert!(crate::db::posts::due_for_subscribers(&site.pool)
        .await
        .is_empty());
}

#[tokio::test]
async fn the_owner_sees_and_removes_subscribers() {
    let mut site = site_with_subscriptions().await;
    sqlx::query(
        "INSERT INTO subscribers (email, token, confirmed_at) VALUES ('fan@example.com', 't1', CURRENT_TIMESTAMP),
                                                                     ('maybe@example.com', 't2', NULL)",
    )
    .execute(&site.pool)
    .await
    .unwrap();
    let settings = site.get("/admin/settings").await.body;
    assert!(settings.contains("fan@example.com"));
    assert!(
        !settings.contains("maybe@example.com"),
        "only confirmed ones"
    );

    let id: i64 = sqlx::query_scalar("SELECT id FROM subscribers WHERE token = 't1'")
        .fetch_one(&site.pool)
        .await
        .unwrap();
    let request = axum::http::Request::delete(format!("/admin/subscribers/{id}"))
        .header("sec-fetch-site", "same-origin");
    let removed = site.send(request, axum::body::Body::empty()).await;
    assert_eq!(removed.status, StatusCode::OK);
    assert!(subscriber(&site, "fan@example.com").await.is_none());
}
