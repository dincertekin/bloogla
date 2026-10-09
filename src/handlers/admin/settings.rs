//! The settings page: site details, reading options and email.
//! Each card on the page saves separately.

use super::{alert, saved_and_reload};
use crate::app::models::CurrentUser;
use crate::app::state::AppState;
use crate::content::text::escape_html;
use crate::db::or_log;
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
    /// Places on the site the menu can link to: `(name, address)`.
    pub menu_suggestions: Vec<(String, String)>,
    pub me: CurrentUser,
    pub blog_description: String,
    pub posts_per_page: String,
    pub nav_menu: String,
    pub show_views: bool,
    pub email: EmailSettings,
    pub comments: String,
    pub publisher_name: String,
    pub publisher_type: String,
    pub language: String,
    pub languages: Vec<Lang>,
    /// The site's time zone, e.g. `Europe/Istanbul`, and every choice.
    pub timezone: String,
    pub timezones: Vec<&'static str>,
    pub site_icon: String,
    /// Settings → Updates.
    pub update: super::updates::UpdateStatus,
}

impl SettingsTemplate {
    /// True for the site's current time zone (to preselect it).
    fn is_timezone(&self, zone: &str) -> bool {
        self.timezone == zone
    }
}

/// Mail server and email options shown in Settings.
pub struct EmailSettings {
    pub host: String,
    pub port: String,
    pub security: String,
    pub username: String,
    pub has_password: bool,
    pub from: String,
    pub notify_comments: bool,
}

/// Settings cards submit separately; absent fields are left unchanged.
#[derive(Deserialize)]
pub struct GeneralSettingsForm {
    pub blog_name: Option<String>,
    pub blog_description: Option<String>,
    pub posts_per_page: Option<String>,
    pub nav_menu: Option<String>,
    /// Checkbox: show view counts on the public site.
    pub show_views: Option<String>,
    /// `off`, `moderated` or `open`.
    pub comments: Option<String>,
    pub publisher_name: Option<String>,
    pub publisher_type: Option<String>,
    /// URL of the favicon, usually from the media library.
    pub site_icon: Option<String>,
    /// Site language code (`en`, `tr`).
    pub language: Option<String>,
    /// Time zone name, e.g. `Europe/Istanbul`.
    pub timezone: Option<String>,
}

/// Every time zone, by name (`Africa/Abidjan` … `UTC`). Old aliases like
/// `US/Eastern` are left out, so each place is listed once.
pub fn timezone_names() -> Vec<&'static str> {
    let mut names: Vec<&'static str> = chrono_tz::TZ_VARIANTS
        .iter()
        .map(|tz| tz.name())
        .filter(|name| name.contains('/') && !name.starts_with("Etc/"))
        .filter(|name| {
            let area = name.split('/').next().unwrap_or_default();
            matches!(
                area,
                "Africa"
                    | "America"
                    | "Antarctica"
                    | "Asia"
                    | "Atlantic"
                    | "Australia"
                    | "Europe"
                    | "Indian"
                    | "Pacific"
            )
        })
        .collect();
    names.sort_unstable();
    names.push("UTC");
    names
}

/// GET /admin/settings -> Settings page.
pub async fn page(
    State(state): State<AppState>,
    Extension(me): Extension<CurrentUser>,
) -> impl IntoResponse {
    let site = settings::load(&state.pool).await;
    let mut menu_suggestions = vec![(me.t("Home").to_string(), "/".to_string())];
    for page in crate::db::posts::public_pages(&state.pool).await {
        let url = crate::server::routes::post_path(&page.slug, true);
        menu_suggestions.push((page.title, url));
    }
    for tag in crate::db::tags::all(&state.pool).await {
        menu_suggestions.push((tag.name, format!("/tag/{}", tag.slug)));
    }
    SettingsTemplate {
        blog_name: site.blog_name.clone(),
        menu_suggestions,
        blog_description: site.blog_description.clone(),
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
            notify_comments: site.notify_comments,
        },
        comments: site.comments.as_str().to_string(),
        publisher_name: site.publisher_name.clone(),
        publisher_type: site.publisher_type.clone(),
        language: site.language.code().to_string(),
        languages: Lang::all(),
        timezone: site.timezone.name().to_string(),
        timezones: timezone_names(),
        site_icon: site.site_icon.clone(),
        update: super::updates::UpdateStatus::load(&state, &site, &me, None),
        me,
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
        if let Some(zone) = form.timezone.as_deref() {
            if zone.parse::<chrono_tz::Tz>().is_err() {
                return settings_error(me.lang, "Choose a time zone from the list.");
            }
            values.push(("timezone", zone.to_string()));
        }
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
                "Each menu link needs a name and an address that starts with / or https://.",
            );
        }
        values.extend([
            ("posts_per_page", posts_per_page),
            ("nav_menu", nav_menu),
            ("show_views", form.show_views.is_some().to_string()),
            (
                "comments",
                CommentMode::parse(form.comments.as_deref().unwrap_or_default())
                    .as_str()
                    .to_string(),
            ),
        ]);
    }

    // People who haven't picked their own language see the admin panel in
    // the site's, so for them a new site language reloads the page.
    let new_language = values
        .iter()
        .find(|(key, _)| *key == "language")
        .and_then(|(_, code)| Lang::parse(code));
    let saved = match new_language {
        Some(lang) if lang != me.lang && follows_site_language(&state, me.id).await => {
            saved_and_reload(lang)
        }
        _ => alert(me.lang, "success", "Saved."),
    };
    save(&state, me.lang, &values, saved).await
}

/// Whether this person uses the site's language for the admin panel.
async fn follows_site_language(state: &AppState, user_id: i64) -> bool {
    let own: Option<String> = or_log(
        sqlx::query_scalar("SELECT language FROM users WHERE id = ?")
            .bind(user_id)
            .fetch_optional(&state.pool)
            .await,
        "load language",
    );
    own.is_some_and(|code| code.is_empty())
}

fn settings_error(lang: Lang, message: &str) -> Response {
    alert(lang, "error", message)
}

/// Save a settings card's values together; answer with `saved` if that worked.
async fn save(
    state: &AppState,
    lang: Lang,
    values: &[(&str, String)],
    saved: Response,
) -> Response {
    match settings::save(&state.pool, values).await {
        Ok(()) => saved,
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
        (
            "notify_comments",
            form.notify_comments.is_some().to_string(),
        ),
    ];
    if !form.smtp_password.is_empty() {
        values.push(("smtp_password", form.smtp_password));
    }
    save(
        &state,
        me.lang,
        &values,
        alert(me.lang, "success", "Saved."),
    )
    .await
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
