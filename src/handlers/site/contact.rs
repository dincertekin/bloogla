//! The contact form: `[contact-form]` in a post or page shows it, and the
//! messages visitors send with it are saved for Admin → Messages (and
//! emailed to admins and editors when email is set up).

use crate::app::state::AppState;
use crate::content::text::escape_html;
use crate::db::or_log;
use crate::db::posts::PUBLIC_POST_FILTER;
use crate::db::settings;
use crate::i18n::Lang;
use crate::server::routes::post_path;

use axum::extract::{ConnectInfo, State};
use axum::http::{HeaderMap, StatusCode};
use axum::response::{IntoResponse, Redirect, Response};
use axum_extra::extract::Form;
use serde::Deserialize;
use std::net::SocketAddr;

use crate::content::shortcodes::CONTACT_FORM as MARKER;

/// People take longer than this between opening the page and sending.
const MIN_SECONDS_TO_WRITE: i64 = 3;

/// `html` with the contact form where `[contact-form]` was, for the post
/// or page `post_id` (`None`: the editor's preview, where the form can't be
/// sent). Only the first one is kept: the page links back to it by its id.
pub fn with_form(html: &str, lang: Lang, post_id: Option<i64>, notice: Option<&str>) -> String {
    let Some((before, marker, after)) = find_marker(html) else {
        return html.to_string();
    };
    // The writer's heading and button text, or the usual words in the
    // site's language.
    let mut parts = marker.split(':').map(|hex| {
        crate::app::security::from_hex(hex)
            .and_then(|bytes| String::from_utf8(bytes).ok())
            .filter(|text| !text.is_empty())
    });
    let heading = parts
        .next()
        .flatten()
        .unwrap_or_else(|| lang.t("Send a message").to_string());
    let send = parts
        .next()
        .flatten()
        .unwrap_or_else(|| lang.t("Send").to_string());

    // When the page was opened, encrypted so it can't be made up.
    let opened = crate::app::secrets::encrypt(&chrono::Utc::now().timestamp().to_string());
    // The preview sits inside the editor's own form, where browsers drop a
    // second <form>: it gets a plain box and a button that does nothing.
    let (post_id, open, close, button) = match post_id {
        Some(id) => (
            id.to_string(),
            r#"<form class="comment-form contact-form" method="POST">"#,
            "</form>",
            r#"type="submit""#,
        ),
        None => (
            String::new(),
            r#"<div class="comment-form contact-form">"#,
            "</div>",
            r#"type="button" disabled"#,
        ),
    };
    let notice = notice
        .and_then(notice_text)
        .map(|text| {
            format!(
                r#"<p class="comment-notice" role="status">{}</p>"#,
                lang.t(text)
            )
        })
        .unwrap_or_default();
    let t = |text: &'static str| escape_html(lang.t(text));
    let form = format!(
        r#"<div class="contact" id="contact">{notice}{open}
<h2>{heading}</h2>
<div class="comment-fields">
<label>{name}<input type="text" name="name" required maxlength="80" autocomplete="name"></label>
<label>{email}<input type="email" name="email" required maxlength="200" autocomplete="email" placeholder="{privacy}"></label>
</div>
<label>{message}<textarea name="message" rows="6" required minlength="2" maxlength="5000"></textarea></label>
<div class="comment-trap" aria-hidden="true"><label>Website <input type="text" name="website" tabindex="-1" autocomplete="off"></label></div>
<input type="hidden" name="page" value="{post_id}">
<input type="hidden" name="opened" value="{opened}">
<button {button}>{send}</button>
{close}</div>"#,
        heading = escape_html(&heading),
        name = t("Name"),
        email = t("Email"),
        message = t("Message"),
        privacy = t("Only used to reply to you"),
        opened = escape_html(&opened),
        send = escape_html(&send),
    );
    // A second form on the same page is left out.
    let mut rest = after.to_string();
    while let Some((start, _, end)) = find_marker(&rest) {
        rest = format!("{start}{end}");
    }
    format!("{before}{form}{rest}")
}

/// The first contact form marker in `html`: the text before it, what it
/// carries (`hex:hex`), and the text after it.
fn find_marker(html: &str) -> Option<(&str, &str, &str)> {
    let start = html.find(MARKER)?;
    let inside = &html[start + MARKER.len()..];
    let end = inside.find("-->")?;
    Some((&html[..start], &inside[..end], &inside[end + 3..]))
}

/// The message shown above the form after sending (`?contact=…`).
fn notice_text(code: &str) -> Option<&'static str> {
    match code {
        "sent" => Some("Thank you! Your message has been sent."),
        "invalid" => Some(
            "Please fill in your name, your email address and a message (up to 5,000 characters).",
        ),
        "slow" => Some("You've sent several messages in a short time. Please wait a few minutes."),
        _ => None,
    }
}

#[derive(Deserialize)]
pub struct ContactForm {
    name: String,
    email: String,
    message: String,
    /// The post or page the form is on.
    page: i64,
    /// Hidden field; people never fill it in, many bots do.
    #[serde(default)]
    website: String,
    /// When the page was opened, encrypted (see [`with_form`]).
    #[serde(default)]
    opened: String,
}

/// POST to the page's own address -> Save a visitor's message and go back
/// to the page.
pub async fn submit(
    State(state): State<AppState>,
    ConnectInfo(peer): ConnectInfo<SocketAddr>,
    headers: HeaderMap,
    Form(form): Form<ContactForm>,
) -> Response {
    // Only a public post or page that has a contact form takes messages.
    let found: Option<(String, bool, String)> = or_log(
        sqlx::query_as(&format!(
            "SELECT slug, is_page, title FROM posts
             WHERE id = ? AND content LIKE '%[contact-form%' AND {PUBLIC_POST_FILTER}"
        ))
        .bind(form.page)
        .fetch_optional(&state.pool)
        .await,
        "find the contact form's page",
    );
    let Some((slug, is_page, title)) = found else {
        return StatusCode::NOT_FOUND.into_response();
    };
    let site = settings::load(&state.pool).await;
    let path = if is_page && site.home_page == Some(form.page) {
        "/".to_string()
    } else {
        post_path(&slug, is_page)
    };
    let back = |code: &str| Redirect::to(&format!("{path}?contact={code}#contact")).into_response();

    // Bots: act as if it worked, keep nothing. A form without a real
    // opening time didn't come from the page.
    let opened =
        crate::app::secrets::decrypt_sealed(&form.opened).and_then(|t| t.parse::<i64>().ok());
    let too_fast = opened.is_none_or(|t| chrono::Utc::now().timestamp() - t < MIN_SECONDS_TO_WRITE);
    if !form.website.trim().is_empty() || too_fast {
        return back("sent");
    }

    let name = form.name.trim();
    let email = form.email.trim();
    let message = form.message.trim();
    let valid = (1..=80).contains(&name.chars().count())
        && (3..=200).contains(&email.len())
        && email.contains('@')
        && !email.contains(char::is_whitespace)
        && (2..=5000).contains(&message.chars().count());
    if !valid {
        return back("invalid");
    }
    if !super::allow_from(&state, peer, &headers) {
        return back("slow");
    }

    let saved =
        sqlx::query("INSERT INTO messages (post_id, name, email, message) VALUES (?, ?, ?, ?)")
            .bind(form.page)
            .bind(name)
            .bind(email)
            .bind(message)
            .execute(&state.pool)
            .await;
    if let Err(e) = saved {
        tracing::error!("Failed to save a message: {e}");
        return StatusCode::INTERNAL_SERVER_ERROR.into_response();
    }

    crate::app::console::activity_tv(
        crate::app::console::Kind::Note,
        "A visitor sent a message from “{title}”.",
        &title,
    );
    crate::services::email::notify_new_message(
        &state,
        crate::services::email::NewMessage {
            page_title: title,
            name: name.to_string(),
            email: email.to_string(),
            text: message.to_string(),
        },
    );
    back("sent")
}
