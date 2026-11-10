//! Email subscriptions: the confirmation email a new subscriber gets, and
//! sending a new post to everyone who confirmed.
//!
//! A post goes out when it's public and its "Email this post to
//! subscribers" box was ticked in the editor ([`send_due_posts`] looks every
//! minute, so scheduled posts go out when they appear). Each post is sent
//! once at most.

use crate::app::state::AppState;
use crate::content::markdown::excerpt;
use crate::content::text::escape_html;
use crate::db::{posts, settings, subscribers};
use crate::server::routes::post_path;
use crate::services::email::{self, Email};

/// Characters of the post shown in the email, before "Read the post".
const EXCERPT_CHARS: usize = 300;

/// The address of someone's confirm link and unsubscribe link.
pub fn confirm_url(state: &AppState, token: &str) -> String {
    format!("{}/subscribe/{token}", state.config.base_url)
}

pub fn unsubscribe_url(state: &AppState, token: &str) -> String {
    format!("{}/unsubscribe/{token}", state.config.base_url)
}

/// Ask a new subscriber to confirm, in the background (the visitor doesn't
/// wait for the mail server).
pub fn send_confirmation(state: &AppState, to: String, token: String) {
    let state = state.clone();
    tokio::spawn(async move {
        let site = settings::load(&state.pool).await;
        let lang = site.language;
        let link = confirm_url(&state, &token);
        let ask = lang.tv(
            "Please confirm that you'd like new posts from {site} by email.",
            &site.blog_name,
        );
        let ignore =
            lang.t("If you didn't ask for this, ignore this email: you won't get any more.");
        let email = Email {
            to,
            subject: lang.tv("Confirm your subscription to {site}", &site.blog_name),
            text: format!("{ask}\n\n{link}\n\n{ignore}\n"),
            html: email::layout(
                &site.blog_name,
                &format!(
                    r#"<p style="margin:0 0 20px">{}</p>
<p style="margin:0"><a href="{link}" style="display:inline-block;background:#111827;color:#ffffff;text-decoration:none;padding:10px 16px;border-radius:8px;font-size:14px">{}</a></p>"#,
                    escape_html(&ask),
                    lang.t("Confirm subscription"),
                    link = escape_html(&link),
                ),
                ignore,
            ),
            reply_to: None,
            unsubscribe: None,
        };
        if let Err(e) = email::send(&state, &email).await {
            tracing::warn!("Subscription confirmation to {} failed: {e}", email.to);
        }
    });
}

/// Look for posts to send every minute, so a scheduled post goes out soon
/// after it appears. Each round is one small query when nothing is due.
pub fn spawn_sender(state: AppState) {
    tokio::spawn(async move {
        let mut interval = tokio::time::interval(std::time::Duration::from_secs(60));
        loop {
            interval.tick().await;
            send_due_posts(&state).await;
        }
    });
}

/// Send every post that's due to everyone who confirmed.
pub async fn send_due_posts(state: &AppState) {
    let due = posts::due_for_subscribers(&state.pool).await;
    if due.is_empty() {
        return;
    }
    let site = settings::load(&state.pool).await;
    let can_send = site.subscriptions_on && email::smtp_settings(state).await.is_some();
    for post in due {
        // Marked first, so a post is never sent twice, even if sending
        // stops halfway (Bloogla restarting, say).
        if !posts::mark_emailed(&state.pool, post.id).await || !can_send {
            continue;
        }
        let readers = subscribers::confirmed(&state.pool).await;
        crate::app::console::activity_tv(
            crate::app::console::Kind::Note,
            "Sending “{title}” to subscribers.",
            &post.title,
        );
        let lang = site.language;
        let link = format!("{}{}", state.config.base_url, post_path(&post.slug, false));
        let summary = excerpt(&post.content, EXCERPT_CHARS);
        let read = lang.t("Read the post");
        let why = lang.tv(
            "You get this because you subscribed to {site}.",
            &site.blog_name,
        );
        let stop = lang.t("Unsubscribe");
        for reader in readers {
            let unsubscribe = unsubscribe_url(state, &reader.token);
            let email = Email {
                to: reader.email,
                subject: post.title.clone(),
                text: format!(
                    "{}\n\n{summary}\n\n{read}: {link}\n\n--\n{why}\n{stop}: {unsubscribe}\n",
                    post.title
                ),
                html: email::layout(
                    &site.blog_name,
                    &format!(
                        r#"<h1 style="margin:0 0 12px;font-size:22px;line-height:1.3">{title}</h1>
<p style="margin:0 0 20px;color:#374151">{summary}</p>
<p style="margin:0"><a href="{link}" style="display:inline-block;background:#111827;color:#ffffff;text-decoration:none;padding:10px 16px;border-radius:8px;font-size:14px">{read}</a></p>"#,
                        title = escape_html(&post.title),
                        summary = escape_html(&summary),
                        link = escape_html(&link),
                        read = escape_html(read),
                    ),
                    &format!(
                        r#"{} <a href="{}" style="color:#9ca3af">{}</a>"#,
                        escape_html(&why),
                        escape_html(&unsubscribe),
                        escape_html(stop)
                    ),
                ),
                reply_to: None,
                unsubscribe: Some(unsubscribe),
            };
            if let Err(e) = email::send(state, &email).await {
                tracing::warn!("Sending “{}” to {} failed: {e}", post.title, email.to);
            }
        }
    }
}
