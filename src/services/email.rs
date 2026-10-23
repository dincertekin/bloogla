//! Outgoing email over SMTP: comment and message notifications, and test
//! emails.
//!
//! The mail server is configured in Settings (host, port, security, login,
//! sender). Nothing is sent until that's filled in.

use crate::app::state::AppState;
use crate::content::text::escape_html;
use crate::db::{or_log, settings};

use lettre::message::{Mailbox, MultiPart};
use lettre::transport::smtp::authentication::Credentials;
use lettre::{AsyncSmtpTransport, AsyncTransport, Message, Tokio1Executor};
use std::time::Duration;

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
    /// Where replies go, when not to the sender (a visitor's message).
    pub reply_to: Option<String>,
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

    let mut builder = Message::builder().from(from).to(to);
    if let Some(reply_to) = email.reply_to.as_deref().and_then(|a| a.parse().ok()) {
        builder = builder.reply_to(reply_to);
    }
    builder
        .subject(email.subject.clone())
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
                reply_to: None,
            };
            if let Err(e) = send(&state, &email).await {
                tracing::warn!("Comment notification to {} failed: {e}", email.to);
            }
        }
    });
}

/// A message a visitor sent with the contact form.
pub struct NewMessage {
    /// The post or page the form is on.
    pub page_title: String,
    pub name: String,
    pub email: String,
    pub text: String,
}

/// Email a visitor's message to admins and editors (who read Messages), in
/// the background. Replying answers the visitor. Nothing is sent when email
/// isn't set up: the message is in Messages either way.
pub fn notify_new_message(state: &AppState, message: NewMessage) {
    let state = state.clone();
    tokio::spawn(async move {
        if smtp_settings(&state).await.is_none() {
            return;
        }
        let site = settings::load(&state.pool).await;
        let recipients: Vec<String> = or_log(
            sqlx::query_scalar("SELECT email FROM users WHERE role IN ('admin', 'editor')")
                .fetch_all(&state.pool)
                .await,
            "message notification recipients",
        );
        let lang = site.language;
        let messages_url = format!("{}/admin/messages", state.config.base_url);
        let subject = lang.tv("New message from {name}", &message.name);
        let intro = |name: String, title: String| {
            lang.tv2("{name} sent a message from “{title}”:", name, title)
        };
        let text = format!(
            "{} ({})\n\n{}\n\n{} {messages_url}\n",
            intro(message.name.clone(), message.page_title.clone()),
            message.email,
            message.text,
            lang.t("Reply to this email to answer. All messages:")
        );
        let html = layout(
            &site.blog_name,
            &format!(
                r#"<p style="margin:0 0 12px">{} <a href="mailto:{email}" style="color:#374151">{email}</a></p>
<blockquote style="margin:0 0 20px;padding-left:12px;border-left:3px solid #e5e7eb;color:#374151">{}</blockquote>
<p style="margin:0 0 12px">{}</p>
<p style="margin:0"><a href="{messages_url}" style="display:inline-block;background:#111827;color:#ffffff;text-decoration:none;padding:10px 16px;border-radius:8px;font-size:14px">{}</a></p>"#,
                intro(
                    format!("<strong>{}</strong>", escape_html(&message.name)),
                    format!("<strong>{}</strong>", escape_html(&message.page_title)),
                ),
                crate::handlers::site::comments::paragraphs(&message.text),
                lang.t("Reply to this email to answer."),
                lang.t("Open Messages"),
                email = escape_html(&message.email),
            ),
            lang.t("You get this because you can read messages from the contact form."),
        );
        for to in recipients {
            let email = Email {
                to,
                subject: subject.clone(),
                text: text.clone(),
                html: html.clone(),
                reply_to: Some(message.email.clone()),
            };
            if let Err(e) = send(&state, &email).await {
                tracing::warn!("Message notification to {} failed: {e}", email.to);
            }
        }
    });
}
