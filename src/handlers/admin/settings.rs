//! The settings page: site details, reading options and email.
//! Each card on the page saves separately.

use super::alert;
use crate::app::models::CurrentUser;
use crate::app::state::AppState;
use crate::content::text::escape_html;
use crate::db::settings::{self, CommentMode};
use crate::i18n::Lang;

use askama::Template;
use axum::extract::{Extension, State};
use axum::response::{IntoResponse, Response};
use axum_extra::extract::Form;
use serde::Deserialize;

#[derive(Template)]
#[template(path = "settings.html")]
pub struct SettingsTemplate {
    pub blog_name: String,
    pub me: CurrentUser,
    pub blog_description: String,
    pub blog_keywords: String,
    pub posts_per_page: String,
    pub nav_menu: String,
    pub show_views: bool,
    pub email: EmailSettings,
    pub comments: String,
    pub send_webmentions: bool,
    pub publisher_name: String,
    pub publisher_type: String,
    pub language: String,
    pub languages: Vec<Lang>,
    pub site_icon: String,
}

/// Mail server and email options shown in Settings.
pub struct EmailSettings {
    pub host: String,
    pub port: String,
    pub security: String,
    pub username: String,
    pub has_password: bool,
    pub from: String,
    pub newsletter: bool,
    pub notify_comments: bool,
}

/// Settings cards submit separately; absent fields are left unchanged.
#[derive(Deserialize)]
pub struct GeneralSettingsForm {
    pub blog_name: Option<String>,
    pub blog_description: Option<String>,
    pub blog_keywords: Option<String>,
    pub posts_per_page: Option<String>,
    pub nav_menu: Option<String>,
    /// Checkbox: show view counts on the public site.
    pub show_views: Option<String>,
    /// `off`, `moderated` or `open`.
    pub comments: Option<String>,
    /// Checkbox: notify sites that posts link to.
    pub send_webmentions: Option<String>,
    pub publisher_name: Option<String>,
    pub publisher_type: Option<String>,
    /// URL of the favicon, usually from the media library.
    pub site_icon: Option<String>,
    /// Site language code (`en`, `tr`).
    pub language: Option<String>,
}

/// GET /admin/settings -> Settings page.
pub async fn page(
    State(state): State<AppState>,
    Extension(me): Extension<CurrentUser>,
) -> impl IntoResponse {
    let site = settings::load(&state.pool).await;
    SettingsTemplate {
        blog_name: site.blog_name.clone(),
        me,
        blog_description: site.blog_description.clone(),
        blog_keywords: site.blog_keywords.clone(),
        posts_per_page: site.posts_per_page.to_string(),
        nav_menu: site.nav_menu.clone(),
        show_views: site.show_views,
        email: EmailSettings {
            host: site.smtp_host.clone(),
            port: site.smtp_port.to_string(),
            security: site.smtp_security.clone(),
            username: site.smtp_username.clone(),
            has_password: !site.smtp_password.is_empty(),
            from: site.smtp_from.clone(),
            newsletter: site.newsletter,
            notify_comments: site.notify_comments,
        },
        comments: site.comments.as_str().to_string(),
        send_webmentions: site.send_webmentions,
        publisher_name: site.publisher_name.clone(),
        publisher_type: site.publisher_type.clone(),
        language: site.language.code().to_string(),
        languages: Lang::all(),
        site_icon: site.site_icon.clone(),
    }
}

/// POST /admin/settings/general -> Update general blog settings via HTMX.
pub async fn update_general(
    State(state): State<AppState>,
    Extension(me): Extension<CurrentUser>,
    Form(form): Form<GeneralSettingsForm>,
) -> impl IntoResponse {
    let mut values: Vec<(&str, String)> = Vec::new();

    // "Site" card
    if let Some(blog_name) = form.blog_name {
        let blog_name = blog_name.trim().to_string();
        if blog_name.is_empty() {
            return settings_error(me.lang, "Give your site a title.");
        }
        let publisher_type = match form.publisher_type.as_deref() {
            Some("Organization") => "Organization",
            _ => "Person",
        };
        let site_icon = form.site_icon.unwrap_or_default().trim().to_string();
        if !(site_icon.is_empty()
            || site_icon.starts_with("/uploads/")
            || site_icon.starts_with("https://"))
        {
            return settings_error(me.lang, "Choose the site icon from your media library.");
        }
        values.extend([
            ("blog_name", blog_name),
            (
                "blog_description",
                form.blog_description.unwrap_or_default().trim().to_string(),
            ),
            (
                "blog_keywords",
                form.blog_keywords.unwrap_or_default().trim().to_string(),
            ),
            (
                "publisher_name",
                form.publisher_name.unwrap_or_default().trim().to_string(),
            ),
            ("publisher_type", publisher_type.to_string()),
            (
                "language",
                form.language
                    .as_deref()
                    .and_then(Lang::parse)
                    .unwrap_or_default()
                    .code()
                    .to_string(),
            ),
            ("site_icon", site_icon),
        ]);
    }

    // "Reading" card (always sends posts_per_page)
    if let Some(per_page) = form.posts_per_page {
        let posts_per_page = match per_page.trim() {
            "" => "10".to_string(),
            v => match v.parse::<u32>() {
                Ok(n @ 1..=100) => n.to_string(),
                _ => {
                    return settings_error(
                        me.lang,
                        "Posts per page must be a number from 1 to 100.",
                    )
                }
            },
        };
        let nav_menu = form.nav_menu.unwrap_or_default().trim().to_string();
        let menu_lines = nav_menu.lines().filter(|l| !l.trim().is_empty()).count();
        if settings::parse_menu(&nav_menu).len() != menu_lines {
            return settings_error(
                me.lang,
                "Each menu line must look like <code>Label | /path</code> or \
                 <code>Label | https://example.com</code>.",
            );
        }
        values.extend([
            ("posts_per_page", posts_per_page),
            ("nav_menu", nav_menu),
            ("show_views", form.show_views.is_some().to_string()),
            (
                "send_webmentions",
                form.send_webmentions.is_some().to_string(),
            ),
            (
                "comments",
                CommentMode::parse(form.comments.as_deref().unwrap_or_default())
                    .as_str()
                    .to_string(),
            ),
        ]);
    }

    save(&state, me.lang, &values).await
}

fn settings_error(lang: Lang, message: &str) -> Response {
    alert(lang, "error", message)
}

/// Save a settings card's values together and report the result.
async fn save(state: &AppState, lang: Lang, values: &[(&str, String)]) -> Response {
    match settings::save(&state.pool, values).await {
        Ok(()) => alert(lang, "success", "Saved."),
        Err(e) => {
            tracing::error!("Failed to save settings: {e}");
            settings_error(lang, "Couldn't save your changes. Please try again.")
        }
    }
}

#[derive(Deserialize)]
pub struct EmailSettingsForm {
    smtp_host: String,
    smtp_port: String,
    smtp_security: String,
    #[serde(default)]
    smtp_username: String,
    /// Left empty to keep the saved password.
    #[serde(default)]
    smtp_password: String,
    smtp_from: String,
    newsletter: Option<String>,
    notify_comments: Option<String>,
}

/// POST /admin/settings/email -> Save mail server and email options.
pub async fn update_email(
    State(state): State<AppState>,
    Extension(me): Extension<CurrentUser>,
    Form(form): Form<EmailSettingsForm>,
) -> Response {
    let host = form.smtp_host.trim();
    let from = form.smtp_from.trim();
    let Ok(port) = form.smtp_port.trim().parse::<u16>() else {
        return settings_error(me.lang, "The port must be a number, usually 587 or 465.");
    };
    if !host.is_empty() && !from.contains('@') {
        return settings_error(me.lang, "Add the address emails should come from.");
    }
    if form.newsletter.is_some() && host.is_empty() {
        return settings_error(
            me.lang,
            "Set up the mail server before turning on the newsletter.",
        );
    }
    let security = match form.smtp_security.as_str() {
        "tls" => "tls",
        "none" => "none",
        _ => "starttls",
    };

    let mut values = vec![
        ("smtp_host", host.to_string()),
        ("smtp_port", port.to_string()),
        ("smtp_security", security.to_string()),
        ("smtp_username", form.smtp_username.trim().to_string()),
        ("smtp_from", from.to_string()),
        ("newsletter", form.newsletter.is_some().to_string()),
        (
            "notify_comments",
            form.notify_comments.is_some().to_string(),
        ),
    ];
    if !form.smtp_password.is_empty() {
        values.push(("smtp_password", form.smtp_password));
    }
    save(&state, me.lang, &values).await
}

/// POST /admin/settings/email/test -> Send a test email to yourself.
pub async fn send_test_email(
    State(state): State<AppState>,
    Extension(me): Extension<CurrentUser>,
) -> Response {
    let blog_name = settings::load(&state.pool).await.blog_name.clone();
    let email = crate::services::email::Email {
        to: me.email.clone(),
        subject: me.tv("Test email from {site}", &blog_name),
        text: format!("{}\n", me.t("If you can read this, email is working.")),
        html: crate::services::email::layout(
            &blog_name,
            &format!(
                r#"<p style="margin:0">{}</p>"#,
                me.t("If you can read this, email is working.")
            ),
            "",
        ),
        unsubscribe: None,
    };
    match crate::services::email::send(&state, &email).await {
        Ok(()) => alert(
            me.lang,
            "success",
            &me.tv(
                "Sent to {email}. Check your inbox (and spam folder).",
                escape_html(&me.email),
            ),
        ),
        Err(e) => settings_error(me.lang, &me.tv("Couldn't send: {error}", escape_html(&e))),
    }
}
