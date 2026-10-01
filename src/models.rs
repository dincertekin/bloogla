use crate::config::Config;

use serde::{Deserialize, Serialize};
use sqlx::{Pool, Sqlite};
use std::sync::atomic::AtomicBool;
use std::sync::{Arc, RwLock};
use tera::Tera;

/// Column list shared by every `Post` query. Keep in sync with the struct:
/// a missing column makes `FromRow` fail at runtime, not at compile time.
pub const POST_SELECT: &str = "SELECT id, title, slug, content, cover_image,
        COALESCE(views, 0) AS views, status, is_page,
        COALESCE(published_at, CURRENT_TIMESTAMP) AS published_at,
        COALESCE(created_at, CURRENT_TIMESTAMP) AS created_at
    FROM posts";

/// Posts visitors may see: published or scheduled, with a publish date in the past.
pub const PUBLIC_POST_FILTER: &str =
    "status IN ('published', 'scheduled') AND datetime(published_at) <= datetime('now')";

/// Public posts that appear in listings and feeds (standalone pages are excluded).
pub const LISTED_POST_FILTER: &str = "status IN ('published', 'scheduled') \
    AND datetime(published_at) <= datetime('now') AND is_page = 0";

#[derive(Clone)]
pub struct AppState {
    pub pool: Pool<Sqlite>,
    pub config: Config,
    pub tera: Arc<RwLock<Tera>>,
    /// True until the first admin account exists (browser setup is open).
    pub setup_pending: Arc<AtomicBool>,
}

#[derive(Debug, Clone, sqlx::FromRow, Serialize, Deserialize)]
pub struct Post {
    pub id: i64,
    pub title: String,
    pub slug: String,
    pub content: String,
    pub cover_image: Option<String>,
    pub views: i64,
    pub created_at: String,
    pub status: String,
    pub published_at: String,
    pub is_page: bool,
    #[sqlx(skip)]
    pub reading_time: u32,
    #[sqlx(skip)]
    pub tags: Vec<Tag>,
}

#[derive(Debug, Clone, sqlx::FromRow, Serialize, Deserialize)]
pub struct Tag {
    pub id: i64,
    pub name: String,
    pub slug: String,
}

#[derive(Deserialize)]
pub struct LoginForm {
    pub email: String,
    pub password: String,
}

/// Submitted by the post and page editor.
#[derive(Deserialize)]
pub struct PostForm {
    pub title: String,
    /// Custom URL slug; generated from the title when empty.
    pub slug: Option<String>,
    #[serde(default)]
    pub content: String,
    pub cover_image: Option<String>,
    pub status: Option<String>, // "draft", "published", or "scheduled"
    pub published_at: Option<String>, // Datetime string e.g. "2026-10-01T12:00"
    #[serde(default)]
    pub tag_ids: Vec<i64>,
}

#[derive(Deserialize)]
pub struct CreateTagForm {
    pub name: String,
}

#[derive(Deserialize)]
pub struct SearchQuery {
    pub q: Option<String>,
    pub page: Option<i64>,
}

#[derive(Deserialize)]
pub struct PageQuery {
    pub page: Option<i64>,
}

#[derive(Deserialize)]
/// Settings cards submit separately; absent fields are left unchanged.
pub struct GeneralSettingsForm {
    pub blog_name: Option<String>,
    pub blog_description: Option<String>,
    pub blog_keywords: Option<String>,
    pub posts_per_page: Option<String>,
    pub nav_menu: Option<String>,
    /// Checkbox: show view counts on the public site.
    pub show_views: Option<String>,
    pub publisher_name: Option<String>,
    pub publisher_type: Option<String>,
    /// URL of the favicon, usually from the media library.
    pub site_icon: Option<String>,
}

/// A navigation menu link, configured in Settings.
#[derive(Debug, Clone, Serialize)]
pub struct MenuItem {
    pub label: String,
    pub url: String,
}

/// Page navigation for paginated listings.
#[derive(Debug, Clone, Serialize)]
pub struct Pagination {
    pub current: i64,
    pub total_pages: i64,
    pub prev_url: Option<String>,
    pub next_url: Option<String>,
}

#[derive(Debug, Clone, sqlx::FromRow)]
pub struct Media {
    pub id: i64,
    pub filename: String,
    pub original_name: String,
    pub size_bytes: i64,
    pub width: Option<i64>,
    pub height: Option<i64>,
}

#[derive(Deserialize)]
pub struct UpdateThemeForm {
    pub theme_name: String,
}

#[derive(Deserialize)]
pub struct UpdatePasswordForm {
    pub current_password: String,
    pub new_password: String,
    pub confirm_password: String,
}
