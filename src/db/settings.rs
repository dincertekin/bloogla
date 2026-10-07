//! Site settings, edited in Admin → Settings and stored in the `settings` table.
//!
//! Every setting and its default value is listed in [`Settings`]. Pages read
//! settings on every request, so they are loaded once, kept in memory, and
//! reloaded after one is saved.
//!
//! To add a setting: add a field to [`Settings`], read it in
//! [`Settings::from_rows`], and save it from the settings page.

use crate::app::models::MenuItem;
use crate::i18n::Lang;

use sqlx::SqlitePool;
use std::collections::HashMap;
use std::sync::{Arc, RwLock};

/// How comments work on this site.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CommentMode {
    Off,
    /// Every comment waits for approval.
    Moderated,
    /// Comments appear right away (unless they look like spam).
    Open,
}

impl CommentMode {
    pub fn parse(value: &str) -> Self {
        match value {
            "off" => CommentMode::Off,
            "open" => CommentMode::Open,
            _ => CommentMode::Moderated,
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            CommentMode::Off => "off",
            CommentMode::Moderated => "moderated",
            CommentMode::Open => "open",
        }
    }
}

#[derive(Debug, Clone)]
pub struct Settings {
    // Site
    pub blog_name: String,
    pub blog_description: String,
    pub blog_keywords: String,
    /// Who publishes the site, for search engines.
    pub publisher_name: String,
    /// `Person` or `Organization`.
    pub publisher_type: String,
    /// Favicon URL, usually from the media library.
    pub site_icon: String,
    /// Language of the public site and emails.
    pub language: Lang,
    /// Folder name of the theme in use, e.g. `default`.
    pub active_theme: String,

    // Reading
    pub posts_per_page: i64,
    /// One `Label | URL` per line.
    pub nav_menu: String,
    pub show_views: bool,
    pub comments: CommentMode,
    pub send_webmentions: bool,

    // Email
    pub smtp_host: String,
    pub smtp_port: u16,
    /// `starttls`, `tls` or `none`.
    pub smtp_security: String,
    pub smtp_username: String,
    pub smtp_password: String,
    /// Sender address.
    pub smtp_from: String,
    pub newsletter: bool,
    pub notify_comments: bool,

    /// Theme options (`theme.<theme>.<name>` → value), see
    /// `services/themes/options.rs`.
    pub theme_options: HashMap<String, String>,
}

impl Settings {
    /// Build settings from stored `key → value` rows, using defaults for missing keys.
    fn from_rows(rows: &HashMap<String, String>) -> Self {
        let text = |key: &str, default: &str| {
            rows.get(key)
                .map_or(default, String::as_str)
                .trim()
                .to_string()
        };
        let flag = |key: &str, default: bool| rows.get(key).map_or(default, |v| v == "true");

        Self {
            blog_name: text("blog_name", "Bloogla"),
            blog_description: text("blog_description", ""),
            blog_keywords: text("blog_keywords", ""),
            publisher_name: text("publisher_name", ""),
            publisher_type: text("publisher_type", "Person"),
            site_icon: text("site_icon", ""),
            language: Lang::parse(&text("language", "en")).unwrap_or_default(),
            active_theme: text("active_theme", "default"),

            posts_per_page: text("posts_per_page", "10")
                .parse()
                .unwrap_or(10)
                .clamp(1, 100),
            nav_menu: text("nav_menu", ""),
            show_views: flag("show_views", false),
            comments: CommentMode::parse(&text("comments", "moderated")),
            send_webmentions: flag("send_webmentions", true),

            smtp_host: text("smtp_host", ""),
            smtp_port: text("smtp_port", "587").parse().unwrap_or(587),
            smtp_security: text("smtp_security", "starttls"),
            smtp_username: text("smtp_username", ""),
            // Stored encrypted; if it can't be read (moved without data/secret.key),
            // it's treated as not set and has to be entered again.
            smtp_password: rows
                .get("smtp_password")
                .and_then(|stored| crate::app::secrets::decrypt(stored))
                .unwrap_or_default(),
            smtp_from: text("smtp_from", ""),
            newsletter: flag("newsletter", false),
            notify_comments: flag("notify_comments", true),

            theme_options: rows
                .iter()
                .filter(|(key, _)| key.starts_with("theme."))
                .map(|(key, value)| (key.clone(), value.clone()))
                .collect(),
        }
    }

    /// Navigation menu links.
    pub fn menu(&self) -> Vec<MenuItem> {
        parse_menu(&self.nav_menu)
    }

    /// Publisher name for search engines; falls back to the site name and is
    /// never an email address.
    pub fn publisher(&self) -> &str {
        if self.publisher_name.is_empty() || self.publisher_name.contains('@') {
            &self.blog_name
        } else {
            &self.publisher_name
        }
    }
}

static CACHE: RwLock<Option<Arc<Settings>>> = RwLock::new(None);

/// Current settings (from memory after the first call).
pub async fn load(pool: &SqlitePool) -> Arc<Settings> {
    if let Some(settings) = CACHE.read().ok().and_then(|cache| cache.clone()) {
        return settings;
    }

    match sqlx::query_as::<_, (String, String)>("SELECT key, value FROM settings")
        .fetch_all(pool)
        .await
    {
        Ok(rows) => {
            let settings = Arc::new(Settings::from_rows(&rows.into_iter().collect()));
            if let Ok(mut cache) = CACHE.write() {
                *cache = Some(settings.clone());
            }
            settings
        }
        Err(e) => {
            // Not cached, so the next request tries the database again.
            tracing::error!("Database error loading settings: {e}");
            Arc::new(Settings::from_rows(&HashMap::new()))
        }
    }
}

/// Settings stored encrypted (see `app/secrets.rs`).
const SECRET_KEYS: &[&str] = &["smtp_password"];

/// Save several settings at once (all or nothing), then refresh the cache.
pub async fn save(pool: &SqlitePool, values: &[(&str, String)]) -> Result<(), sqlx::Error> {
    let mut tx = pool.begin().await?;
    for (key, value) in values {
        let value = if SECRET_KEYS.contains(key) {
            crate::app::secrets::encrypt(value)
        } else {
            value.clone()
        };
        sqlx::query(
            "INSERT INTO settings (key, value) VALUES (?, ?)
             ON CONFLICT(key) DO UPDATE SET value = excluded.value",
        )
        .bind(key)
        .bind(value)
        .execute(&mut *tx)
        .await?;
    }
    tx.commit().await?;
    invalidate();
    Ok(())
}

/// Drop cached settings so the next read sees the database.
pub fn invalidate() {
    if let Ok(mut cache) = CACHE.write() {
        *cache = None;
    }
}

/// Parse the navigation menu setting: one `Label | URL` per line.
/// Lines with unsafe URLs (e.g. `javascript:`) are left out.
pub fn parse_menu(raw: &str) -> Vec<MenuItem> {
    raw.lines()
        .filter_map(|line| {
            let (label, url) = line.split_once('|')?;
            let (label, url) = (label.trim(), url.trim());
            let safe_url =
                url.starts_with('/') || url.starts_with("https://") || url.starts_with("http://");
            (!label.is_empty() && safe_url).then(|| MenuItem {
                label: label.to_string(),
                url: url.to_string(),
            })
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn menu_rejects_unsafe_urls() {
        let menu = parse_menu("About | /about\nBad | javascript:alert(1)\nGH | https://github.com");
        let urls: Vec<_> = menu.iter().map(|m| m.url.as_str()).collect();
        assert_eq!(urls, ["/about", "https://github.com"]);
    }

    #[test]
    fn missing_or_invalid_values_use_defaults() {
        let rows: HashMap<String, String> = [
            ("posts_per_page", "5000"),
            ("comments", "nonsense"),
            ("smtp_port", "x"),
        ]
        .into_iter()
        .map(|(k, v)| (k.to_string(), v.to_string()))
        .collect();
        let settings = Settings::from_rows(&rows);
        assert_eq!(settings.blog_name, "Bloogla");
        assert_eq!(settings.posts_per_page, 100);
        assert_eq!(settings.comments, CommentMode::Moderated);
        assert_eq!(settings.smtp_port, 587);
        assert!(settings.send_webmentions);
    }
}
