//! Subscribing to new posts by email: the form under each post (when it's on
//! in Settings → Subscribers), the link that confirms, and unsubscribing.
//! Sending is in `services/subscriptions.rs`.

use super::{base_context, noindex_head};
use crate::app::models::Post;
use crate::app::state::AppState;
use crate::content::text::escape_html;
use crate::db::{posts, settings, subscribers};
use crate::i18n::Lang;
use crate::server::routes::post_path;
use crate::services::themes;

use axum::extract::{ConnectInfo, Path, State};
use axum::http::{HeaderMap, StatusCode};
use axum::response::{IntoResponse, Redirect, Response};
use axum_extra::extract::Form;
use serde::Deserialize;
use std::net::SocketAddr;

/// People take longer than this between opening the page and subscribing.
const MIN_SECONDS_TO_TYPE: i64 = 2;

/// The form themes show under a post as `{{ subscribe_form | safe }}`. It
/// uses the comment form's classes, so every theme already styles it.
pub fn form(lang: Lang, post_path: &str, notice: Option<&str>) -> String {
    let notice = notice
        .and_then(notice_text)
        .map(|text| {
            format!(
                r#"<p class="comment-notice" role="status">{}</p>"#,
                escape_html(lang.t(text))
            )
        })
        .unwrap_or_default();
    // When the page was opened, encrypted so it can't be made up.
    let opened = crate::app::secrets::encrypt(&chrono::Utc::now().timestamp().to_string());
    let t = |text: &'static str| escape_html(lang.t(text));
    format!(
        r#"<div class="subscribe" id="subscribe">{notice}<form class="comment-form contact-form subscribe-form" method="POST" action="{action}">
<h2>{heading}</h2>
<label>{email}<input type="email" name="email" required maxlength="200" autocomplete="email"></label>
<div class="comment-trap" aria-hidden="true"><label>Website <input type="text" name="website" tabindex="-1" autocomplete="off"></label></div>
<input type="hidden" name="opened" value="{opened}">
<button type="submit">{button}</button>
<small>{hint}</small>
</form></div>"#,
        heading = t("Get new posts by email"),
        email = t("Email"),
        button = t("Subscribe"),
        hint = t("Unsubscribe any time."),
        opened = escape_html(&opened),
        action = escape_html(&format!("{post_path}/subscribe")),
    )
}

/// The message shown above the form after sending (`?subscribe=…`).
fn notice_text(code: &str) -> Option<&'static str> {
    match code {
        "check" => Some("Almost done: open the email we just sent you to confirm."),
        "invalid" => Some("Please enter your email address."),
        "slow" => Some("Please wait a few minutes and try again."),
        _ => None,
    }
}

#[derive(Deserialize)]
pub struct SubscribeForm {
    email: String,
    /// Hidden field; people never fill it in, many bots do.
    #[serde(default)]
    website: String,
    /// When the page was opened, encrypted (see [`form`]).
    #[serde(default)]
    opened: String,
}

/// POST /post/:slug/subscribe -> Add the address (unconfirmed), email the
/// link that confirms it, and go back to the post.
pub async fn submit(
    State(state): State<AppState>,
    Path(slug): Path<String>,
    ConnectInfo(peer): ConnectInfo<SocketAddr>,
    headers: HeaderMap,
    Form(form): Form<SubscribeForm>,
) -> Response {
    let site = settings::load(&state.pool).await;
    let post = posts::find_public(&state.pool, &slug, false).await;
    let (Some(post), true) = (post, site.subscriptions_on) else {
        return StatusCode::NOT_FOUND.into_response();
    };
    let path = post_path(&post.slug, false);
    let back =
        |code: &str| Redirect::to(&format!("{path}?subscribe={code}#subscribe")).into_response();

    // Bots: act as if it worked, keep nothing.
    let opened =
        crate::app::secrets::decrypt_sealed(&form.opened).and_then(|t| t.parse::<i64>().ok());
    let too_fast = opened.is_none_or(|t| chrono::Utc::now().timestamp() - t < MIN_SECONDS_TO_TYPE);
    if !form.website.trim().is_empty() || too_fast {
        return back("check");
    }

    let email = form.email.trim();
    let valid = (3..=200).contains(&email.len())
        && email.contains('@')
        && !email.contains(char::is_whitespace);
    if !valid {
        return back("invalid");
    }
    if !super::allow_from(&state, peer, &headers) {
        return back("slow");
    }

    let token = crate::app::security::random_hex(24);
    match subscribers::ask(&state.pool, email, &token).await {
        Ok(subscribers::Asked::Confirm(token)) => {
            crate::services::subscriptions::send_confirmation(&state, email.to_string(), token);
        }
        // Said the same way, so the form doesn't tell who's subscribed.
        Ok(subscribers::Asked::AlreadyConfirmed) => {}
        Err(e) => {
            tracing::error!("Failed to save a subscriber: {e}");
            return StatusCode::INTERNAL_SERVER_ERROR.into_response();
        }
    }
    back("check")
}

/// GET /subscribe/:token -> The link in the confirmation email.
pub async fn confirm(State(state): State<AppState>, Path(token): Path<String>) -> Response {
    let lang = settings::load(&state.pool).await.language;
    if !subscribers::confirm(&state.pool, &token).await {
        return message_page(
            &state,
            StatusCode::NOT_FOUND,
            lang.t("This link doesn't work anymore"),
            lang.t("It may be more than a week old. Subscribe again under any post."),
        )
        .await;
    }
    crate::app::console::activity_t(
        crate::app::console::Kind::Note,
        "Someone subscribed to new posts.",
    );
    message_page(
        &state,
        StatusCode::OK,
        lang.t("You're subscribed"),
        lang.t("New posts will come to your inbox. Every email has a link to unsubscribe."),
    )
    .await
}

/// GET /unsubscribe/:token -> Ask before unsubscribing: links in emails are
/// sometimes opened by mail scanners, which must not unsubscribe anyone.
pub async fn unsubscribe_page(
    State(state): State<AppState>,
    Path(token): Path<String>,
) -> Response {
    let lang = settings::load(&state.pool).await.language;
    if !subscribers::token_exists(&state.pool, &token).await {
        return message_page(
            &state,
            StatusCode::OK,
            lang.t("You're unsubscribed"),
            lang.t("You won't get any more emails about new posts."),
        )
        .await;
    }
    let button = format!(
        r#"<form class="comment-form contact-form" method="POST" action="/unsubscribe/{}"><button type="submit">{}</button></form>"#,
        escape_html(&token),
        escape_html(lang.t("Unsubscribe")),
    );
    message_page_html(
        &state,
        StatusCode::OK,
        lang.t("Unsubscribe from new posts?"),
        &format!(
            "<p>{}</p>{button}",
            escape_html(lang.t("You won't get any more emails about new posts."))
        ),
    )
    .await
}

/// POST /unsubscribe/:token -> The button on that page, or the
/// "Unsubscribe" button email apps show (it posts here directly).
pub async fn unsubscribe(State(state): State<AppState>, Path(token): Path<String>) -> Response {
    if let Err(e) = subscribers::unsubscribe(&state.pool, &token).await {
        tracing::error!("Failed to unsubscribe: {e}");
        return StatusCode::INTERNAL_SERVER_ERROR.into_response();
    }
    let lang = settings::load(&state.pool).await.language;
    message_page(
        &state,
        StatusCode::OK,
        lang.t("You're unsubscribed"),
        lang.t("You won't get any more emails about new posts."),
    )
    .await
}

/// A short themed page with a title and one sentence.
async fn message_page(state: &AppState, status: StatusCode, title: &str, text: &str) -> Response {
    message_page_html(
        state,
        status,
        title,
        &format!("<p>{}</p>", escape_html(text)),
    )
    .await
}

/// A short themed page, drawn like a page of the site. `html` must be safe.
async fn message_page_html(
    state: &AppState,
    status: StatusCode,
    title: &str,
    html: &str,
) -> Response {
    let site = settings::load(&state.pool).await;
    let page = Post {
        id: 0,
        title: title.to_string(),
        slug: String::new(),
        content: String::new(),
        cover_image: None,
        cover_width: None,
        cover_height: None,
        cover_small: None,
        cover_thumb: None,
        views: 0,
        created_at: String::new(),
        status: "published".to_string(),
        published_at: String::new(),
        is_page: true,
        author_id: None,
        author_name: None,
        trashed_at: None,
        reading_time: 0,
        tags: Vec::new(),
        url: String::new(),
    };
    let mut context = base_context(state, &site).await;
    // Personal links: search engines stay away.
    context.insert("seo_head", &noindex_head(state, &site));
    context.insert("content_html", html);
    context.insert("post", &page);
    context.insert("comments_enabled", &false);
    context.insert("previous_post", &None::<()>);
    context.insert("next_post", &None::<()>);
    let mut response = themes::render(
        state,
        &["templates/page.html", "templates/post.html"],
        context,
        status,
    )
    .await;
    // The address holds someone's secret link: never kept, never passed on.
    let headers = response.headers_mut();
    headers.insert(
        axum::http::header::CACHE_CONTROL,
        axum::http::HeaderValue::from_static("no-store"),
    );
    headers.insert(
        axum::http::header::REFERRER_POLICY,
        axum::http::HeaderValue::from_static("no-referrer"),
    );
    response
}
