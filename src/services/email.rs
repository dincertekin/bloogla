//! Outgoing email over SMTP: comment notifications and the newsletter.
//!
//! The mail server is configured in Settings (host, port, security, login,
//! sender). Nothing is sent until that's filled in.

use crate::app::state::AppState;
use crate::content::text::escape_html;
use crate::db::{or_log, settings};

use lettre::message::header::{HeaderName, HeaderValue};
use lettre::message::{Mailbox, MultiPart};
use lettre::transport::smtp::authentication::Credentials;
use lettre::{AsyncSmtpTransport, AsyncTransport, Message, Tokio1Executor};
use std::time::Duration;

/// Pause between newsletter emails, to stay within typical provider limits.
const SEND_INTERVAL: Duration = Duration::from_millis(250);

#[derive(Clone, Copy, PartialEq)]
pub enum Security {
    /// Plain connection upgraded with STARTTLS (usually port 587).
    StartTls,
    /// TLS from the start (usually port 465).
    Tls,
    /// No encryption: only for a mail server on the same machine.
    None,
}

#[derive(Clone)]
pub struct SmtpSettings {
    host: String,
    port: u16,
    security: Security,
    username: String,
    password: String,
    from_email: String,
    from_name: String,
}

/// Mail server settings, or `None` when email isn't set up yet.
pub async fn smtp_settings(state: &AppState) -> Option<SmtpSettings> {
    let site = settings::load(&state.pool).await;
    if site.smtp_host.is_empty() || !site.smtp_from.contains('@') {
        return None;
    }
    let security = match site.smtp_security.as_str() {
        "tls" => Security::Tls,
        "none" => Security::None,
        _ => Security::StartTls,
    };
    Some(SmtpSettings {
        host: site.smtp_host.clone(),
        port: site.smtp_port,
        security,
        username: site.smtp_username.clone(),
        password: site.smtp_password.clone(),
        from_name: site.blog_name.clone(),
        from_email: site.smtp_from.clone(),
    })
}

/// One email to one person.
pub struct Email {
    pub to: String,
    pub subject: String,
    pub text: String,
    pub html: String,
    /// Unsubscribe URL, added as one-click `List-Unsubscribe` headers.
    pub unsubscribe: Option<String>,
}

fn mailer(settings: &SmtpSettings) -> Result<AsyncSmtpTransport<Tokio1Executor>, String> {
    let builder = match settings.security {
        Security::Tls => AsyncSmtpTransport::<Tokio1Executor>::relay(&settings.host),
        Security::StartTls => AsyncSmtpTransport::<Tokio1Executor>::starttls_relay(&settings.host),
        Security::None => Ok(AsyncSmtpTransport::<Tokio1Executor>::builder_dangerous(
            settings.host.clone(),
        )),
    }
    .map_err(|e| format!("mail server: {e}"))?;

    let mut builder = builder
        .port(settings.port)
        .timeout(Some(Duration::from_secs(20)));
    if !settings.username.is_empty() {
        builder = builder.credentials(Credentials::new(
            settings.username.clone(),
            settings.password.clone(),
        ));
    }
    Ok(builder.build())
}

fn build_message(settings: &SmtpSettings, email: &Email) -> Result<Message, String> {
    let from = Mailbox::new(
        Some(settings.from_name.clone()),
        settings
            .from_email
            .parse()
            .map_err(|_| "the sender address in Settings isn't valid".to_string())?,
    );
    let to: Mailbox = email
        .to
        .parse()
        .map_err(|_| format!("{} isn't a valid email address", email.to))?;

    let mut builder = Message::builder()
        .from(from)
        .to(to)
        .subject(email.subject.clone());
    if let Some(url) = &email.unsubscribe {
        builder = builder
            .raw_header(HeaderValue::new(
                HeaderName::new_from_ascii_str("List-Unsubscribe"),
                format!("<{url}>"),
            ))
            .raw_header(HeaderValue::new(
                HeaderName::new_from_ascii_str("List-Unsubscribe-Post"),
                "List-Unsubscribe=One-Click".to_string(),
            ));
    }
    builder
        .multipart(MultiPart::alternative_plain_html(
            email.text.clone(),
            email.html.clone(),
        ))
        .map_err(|e| format!("couldn't build the email: {e}"))
}

/// Send one email now.
pub async fn send(state: &AppState, email: &Email) -> Result<(), String> {
    let settings = smtp_settings(state)
        .await
        .ok_or("Email isn't set up yet. Add your mail server in Settings.")?;
    let message = build_message(&settings, email)?;
    mailer(&settings)?
        .send(message)
        .await
        .map(|_| ())
        .map_err(|e| format!("the mail server refused it: {e}"))
}

/// Wrap body HTML in a simple, readable email layout.
pub fn layout(blog_name: &str, body_html: &str, footer_html: &str) -> String {
    format!(
        r#"<!doctype html><html><body style="margin:0;padding:24px;background:#fafafa;font-family:-apple-system,BlinkMacSystemFont,'Segoe UI',Roboto,Helvetica,Arial,sans-serif;color:#111827;line-height:1.6">
<div style="max-width:600px;margin:0 auto;background:#ffffff;border:1px solid #e5e7eb;border-radius:12px;padding:28px">
<p style="margin:0 0 20px;color:#6b7280;font-size:14px;font-weight:600">{}</p>
{body_html}
</div>
<p style="max-width:600px;margin:16px auto 0;color:#9ca3af;font-size:12px;text-align:center">{footer_html}</p>
</body></html>"#,
        escape_html(blog_name)
    )
}

/// Tell admins and editors that a comment is waiting (if enabled in Settings).
pub fn notify_new_comment(state: &AppState, post_title: String, author: String, excerpt: String) {
    let state = state.clone();
    tokio::spawn(async move {
        let site = settings::load(&state.pool).await;
        if !site.notify_comments || smtp_settings(&state).await.is_none() {
            return;
        }
        let recipients: Vec<String> = or_log(
            sqlx::query_scalar("SELECT email FROM users WHERE role IN ('admin', 'editor')")
                .fetch_all(&state.pool)
                .await,
            "comment notification recipients",
        );
        let (lang, blog_name) = (site.language, &site.blog_name);
        let review_url = format!("{}/admin/comments", state.config.base_url);
        let subject = lang.tv("New comment on “{title}”", &post_title);
        let text = format!(
            "{}\n\n{excerpt}\n\n{} {review_url}\n",
            lang.tv2("{author} commented on “{title}”:", &author, &post_title),
            lang.t("Review it:")
        );
        let html = layout(
            blog_name,
            &format!(
                r#"<p style="margin:0 0 12px">{}</p>
<blockquote style="margin:0 0 20px;padding-left:12px;border-left:3px solid #e5e7eb;color:#374151">{}</blockquote>
<p style="margin:0"><a href="{review_url}" style="display:inline-block;background:#111827;color:#ffffff;text-decoration:none;padding:10px 16px;border-radius:8px;font-size:14px">{}</a></p>"#,
                lang.tv2(
                    "{author} commented on “{title}”:",
                    format!("<strong>{}</strong>", escape_html(&author)),
                    format!("<strong>{}</strong>", escape_html(&post_title)),
                ),
                escape_html(&excerpt),
                lang.t("Review comments"),
            ),
            lang.t("You get this because you can moderate comments. Turn it off in Settings."),
        );
        for to in recipients {
            let email = Email {
                to,
                subject: subject.clone(),
                text: text.clone(),
                html: html.clone(),
                unsubscribe: None,
            };
            if let Err(e) = send(&state, &email).await {
                tracing::warn!("Comment notification to {} failed: {e}", email.to);
            }
        }
    });
}

/// Email a published post to every active subscriber, once per post.
/// Returns how many people it's going to.
pub async fn send_post_to_subscribers(state: &AppState, post_id: i64) -> Result<i64, String> {
    let settings = smtp_settings(state)
        .await
        .ok_or("Email isn't set up yet. Add your mail server in Settings.")?;

    let post: Option<(String, String, String, Option<String>)> = or_log(
        sqlx::query_as(&format!(
            "SELECT title, slug, content, cover_image FROM posts
             WHERE id = ? AND is_page = 0 AND {}",
            crate::db::posts::PUBLIC_POST_FILTER
        ))
        .bind(post_id)
        .fetch_optional(&state.pool)
        .await,
        "newsletter post",
    );
    let (title, slug, content, cover) = post.ok_or("Only published posts can be emailed.")?;

    let subscribers: Vec<(String, String)> = or_log(
        sqlx::query_as("SELECT email, token FROM subscribers WHERE status = 'active'")
            .fetch_all(&state.pool)
            .await,
        "newsletter recipients",
    );
    let count = subscribers.len() as i64;

    // Claim the post so it's never sent twice, even if saved again meanwhile.
    let claimed =
        sqlx::query("INSERT OR IGNORE INTO newsletter_sends (post_id, recipients) VALUES (?, ?)")
            .bind(post_id)
            .bind(count)
            .execute(&state.pool)
            .await
            .map_err(|e| e.to_string())?
            .rows_affected();
    if claimed == 0 {
        return Err("This post has already been emailed.".into());
    }

    let base = state.config.base_url.clone();
    let post_url = format!("{base}{}", crate::server::routes::post_path(&slug, false));
    let site = settings::load(&state.pool).await;
    let (lang, blog_name) = (site.language, site.blog_name.clone());
    let body =
        crate::content::absolute_links(&crate::content::render(state, &content).await, &base);
    let cover_html = cover
        .map(|c| {
            format!(
                r#"<img src="{}" alt="" style="width:100%;border-radius:8px;margin:0 0 20px">"#,
                escape_html(&crate::content::seo::absolute_url(&c, &base))
            )
        })
        .unwrap_or_default();
    let text_excerpt = crate::content::markdown::excerpt(&content, 600);

    let state = state.clone();
    tokio::spawn(async move {
        let mailer = match mailer(&settings) {
            Ok(m) => m,
            Err(e) => return tracing::error!("Newsletter for post {post_id} not sent: {e}"),
        };
        let mut delivered = 0i64;
        for (to, token) in subscribers {
            let unsubscribe = format!("{base}/unsubscribe?token={token}");
            let html = layout(
                &blog_name,
                &format!(
                    r#"{cover_html}<h1 style="font-size:24px;line-height:1.3;margin:0 0 16px">{}</h1>
<div style="font-size:16px">{body}</div>
<p style="margin:24px 0 0"><a href="{post_url}" style="color:#111827">{}</a></p>"#,
                    escape_html(&title),
                    lang.t("Read it on the site"),
                ),
                &format!(
                    r#"{} <a href="{unsubscribe}" style="color:#9ca3af">{}</a>"#,
                    lang.tv("You subscribed to {site}.", escape_html(&blog_name)),
                    lang.t("Unsubscribe"),
                ),
            );
            let email = Email {
                to: to.clone(),
                subject: title.clone(),
                text: format!(
                    "{title}\n\n{text_excerpt}\n\n{} {post_url}\n\n{} {unsubscribe}\n",
                    lang.t("Read it:"),
                    lang.t("Unsubscribe:")
                ),
                html,
                unsubscribe: Some(unsubscribe),
            };
            match build_message(&settings, &email) {
                Ok(message) => match mailer.send(message).await {
                    Ok(_) => delivered += 1,
                    Err(e) => tracing::warn!("Newsletter to {to} failed: {e}"),
                },
                Err(e) => tracing::warn!("Newsletter to {to} skipped: {e}"),
            }
            tokio::time::sleep(SEND_INTERVAL).await;
        }
        let _ = sqlx::query("UPDATE newsletter_sends SET delivered = ? WHERE post_id = ?")
            .bind(delivered)
            .bind(post_id)
            .execute(&state.pool)
            .await;
        tracing::info!("Newsletter for post {post_id}: {delivered}/{count} delivered");
    });

    Ok(count)
}
