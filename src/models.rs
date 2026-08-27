use crate::config::Config;

use serde::{Deserialize, Serialize};
use sqlx::{Pool, Sqlite};
use std::sync::{Arc, RwLock};
use tera::Tera;

#[derive(Clone)]
pub struct AppState {
    pub pool: Pool<Sqlite>,
    pub config: Config,
    pub tera: Arc<RwLock<Tera>>,
}

#[derive(Debug, Clone, sqlx::FromRow, Serialize, Deserialize)]
pub struct Article {
    pub id: i64,
    pub title: String,
    pub slug: String,
    pub content: String,
    pub cover_image: Option<String>,
    pub views: i64,
    pub created_at: String,
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

#[derive(Deserialize)]
pub struct CreateArticleForm {
    pub title: String,
    pub content: String,
    pub cover_image: Option<String>,
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
}

#[derive(Deserialize)]
pub struct GeneralSettingsForm {
    pub blog_name: String,
    pub blog_description: String,
    pub blog_keywords: String,
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
