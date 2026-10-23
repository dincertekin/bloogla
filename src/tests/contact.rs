//! The contact form (`[contact-form]`) and Admin → Messages.

use super::TestSite;
use axum::http::StatusCode;

/// A site with a published "Contact" page holding a form; gives the page's id.
async fn site_with_a_contact_page() -> (TestSite, i64) {
    let mut site = TestSite::with_owner().await;
    site.post(
        "/admin/pages",
        &[
            ("title", "Contact"),
            ("content", "Write to us.\n\n[contact-form]"),
            ("status", "published"),
        ],
    )
    .await;
    let id = sqlx::query_scalar("SELECT id FROM posts WHERE slug = 'contact'")
        .fetch_one(&site.pool)
        .await
        .unwrap();
    (site, id)
}

/// The encrypted opening time a page puts in the form, `seconds_ago`.
fn opened(seconds_ago: i64) -> String {
    let at = chrono::Utc::now().timestamp() - seconds_ago;
    crate::app::secrets::encrypt(&at.to_string())
}

/// Send the form from page `id`, as a visitor who took 10 seconds.
async fn send(site: &mut TestSite, id: i64, fields: &[(&str, &str)]) -> super::Page {
    let page = id.to_string();
    let opened = opened(10);
    let mut all = vec![("page", page.as_str()), ("opened", opened.as_str())];
    all.extend_from_slice(fields);
    site.post("/contact", &all).await
}

async fn message_count(site: &TestSite) -> i64 {
    sqlx::query_scalar("SELECT COUNT(*) FROM messages")
        .fetch_one(&site.pool)
        .await
        .unwrap()
}

const AYSE: &[(&str, &str)] = &[
    ("name", "Ayşe"),
    ("email", "ayse@example.com"),
    (
        "message",
        "Do you make cakes for 20 people?\n\nOn Saturday.",
    ),
];

#[tokio::test]
async fn visitors_send_a_message_and_the_owner_reads_it() {
    let (mut site, id) = site_with_a_contact_page().await;
    site.new_visitor();
    let page = site.get("/contact").await.body;
    assert!(page.contains(r#"<form class="comment-form contact-form" method="POST">"#));
    assert!(page.contains("Write to us."));
    assert!(!page.contains("[contact-form]"));

    let sent = send(&mut site, id, AYSE).await;
    assert_eq!(sent.location(), "/contact?contact=sent#contact");
    let thanks = site.get("/contact?contact=sent").await.body;
    assert!(thanks.contains("Thank you! Your message has been sent."));
    assert_eq!(message_count(&site).await, 1);

    // The owner: a count in the menu, then the message, marked new once.
    site.sign_in(super::ADMIN_EMAIL, super::ADMIN_PASSWORD)
        .await;
    let dashboard = site.get("/admin").await.body;
    assert!(dashboard.contains("1 new message"));
    let messages = site.get("/admin/messages").await.body;
    assert!(messages.contains("Ayşe"));
    assert!(messages.contains("<p>Do you make cakes for 20 people?</p><p>On Saturday.</p>"));
    assert!(messages.contains("mailto:ayse%40example.com?subject="));
    assert!(messages.contains(r#"class="badge badge-home">New<"#));
    let again = site.get("/admin/messages").await.body;
    assert!(!again.contains(r#"class="badge badge-home">New<"#));
    assert!(again.contains("0 new messages"));

    let message_id: i64 = sqlx::query_scalar("SELECT id FROM messages")
        .fetch_one(&site.pool)
        .await
        .unwrap();
    let deleted = site
        .send(
            axum::http::Request::delete(format!("/admin/messages/{message_id}"))
                .header("sec-fetch-site", "same-origin"),
            axum::body::Body::empty(),
        )
        .await;
    assert_eq!(deleted.status, StatusCode::OK);
    assert_eq!(message_count(&site).await, 0);
}

#[tokio::test]
async fn bots_are_thanked_but_nothing_is_kept() {
    let (mut site, id) = site_with_a_contact_page().await;
    site.new_visitor();
    let page = id.to_string();
    let quick = opened(1);
    let in_time = opened(10);
    let cases: Vec<Vec<(&str, &str)>> = vec![
        // Filled in the hidden field.
        [
            AYSE,
            &[
                ("opened", in_time.as_str()),
                ("website", "https://spam.example"),
            ],
        ]
        .concat(),
        // Sent a second after opening the page.
        [AYSE, &[("opened", quick.as_str())]].concat(),
        // Made up the opening time, or left it out.
        [AYSE, &[("opened", "1700000000")]].concat(),
        AYSE.to_vec(),
    ];
    for fields in cases {
        let mut all = vec![("page", page.as_str())];
        all.extend(fields);
        let sent = site.post("/contact", &all).await;
        assert_eq!(sent.location(), "/contact?contact=sent#contact");
    }
    assert_eq!(message_count(&site).await, 0);
}

#[tokio::test]
async fn a_message_needs_a_name_an_email_and_some_text() {
    let (mut site, id) = site_with_a_contact_page().await;
    site.new_visitor();
    for fields in [
        [
            ("name", ""),
            ("email", "ayse@example.com"),
            ("message", "Hello"),
        ],
        [
            ("name", "Ayşe"),
            ("email", "not an email"),
            ("message", "Hello"),
        ],
        [
            ("name", "Ayşe"),
            ("email", "ayse@example.com"),
            ("message", " "),
        ],
    ] {
        let sent = send(&mut site, id, &fields).await;
        assert_eq!(sent.location(), "/contact?contact=invalid#contact");
    }
    assert!(site
        .get("/contact?contact=invalid")
        .await
        .body
        .contains("Please fill in your name, your email address and a message"));
    assert_eq!(message_count(&site).await, 0);
}

#[tokio::test]
async fn only_pages_with_a_form_take_messages() {
    let mut site = TestSite::with_owner().await;
    site.publish("No form here", "published").await;
    let id: i64 = sqlx::query_scalar("SELECT id FROM posts")
        .fetch_one(&site.pool)
        .await
        .unwrap();
    site.new_visitor();
    assert_eq!(
        send(&mut site, id, AYSE).await.status,
        StatusCode::NOT_FOUND
    );
    assert_eq!(message_count(&site).await, 0);
}

#[tokio::test]
async fn authors_dont_see_messages() {
    let (mut site, id) = site_with_a_contact_page().await;
    send(&mut site, id, AYSE).await;
    let password = "Author-Pass-123!";
    let hash = crate::app::security::hash_password(password).unwrap();
    sqlx::query("INSERT INTO users (email, password_hash, name, role) VALUES ('author@example.com', ?, 'Author', 'author')")
        .bind(hash)
        .execute(&site.pool)
        .await
        .unwrap();
    site.new_visitor();
    site.sign_in("author@example.com", password).await;
    let refused = site.get("/admin/messages").await;
    assert_ne!(refused.status, StatusCode::OK);
    assert!(!site.get("/admin").await.body.contains("/admin/messages"));
}

#[tokio::test]
async fn the_preview_shows_the_form_but_it_cant_be_sent() {
    let mut site = TestSite::with_owner().await;
    let preview = site
        .post(
            "/admin/preview",
            &[("content", "Write to us.\n\n[contact-form]")],
        )
        .await
        .body;
    // A <form> inside the editor's form would be dropped by the browser.
    assert!(preview.contains(r#"<div class="comment-form contact-form">"#));
    assert!(!preview.contains("<form"));
    assert!(preview.contains(r#"<button type="button" disabled>"#));
}

#[tokio::test]
async fn the_writer_can_change_the_heading_and_button() {
    let mut site = TestSite::with_owner().await;
    site.post(
        "/admin/pages",
        &[
            ("title", "Book"),
            (
                "content",
                r#"[contact-form title="Book a <b>table</b>" button="Reserve"]"#,
            ),
            ("status", "published"),
        ],
    )
    .await;
    let id: i64 = sqlx::query_scalar("SELECT id FROM posts WHERE slug = 'book'")
        .fetch_one(&site.pool)
        .await
        .unwrap();
    site.new_visitor();
    let page = site.get("/book").await.body;
    // HTML in the wording is never drawn as HTML.
    assert!(page.contains("<h2>Book a table</h2>"));
    assert!(!page.contains("<b>table"));
    assert!(page.contains(r#"<button type="submit">Reserve</button>"#));
    // Messages from it are taken like any other.
    assert_eq!(
        send(&mut site, id, AYSE).await.location(),
        "/book?contact=sent#contact"
    );
    assert_eq!(message_count(&site).await, 1);
}
