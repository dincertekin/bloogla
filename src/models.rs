use crate::config::Config;
use serde::Deserialize;
use sqlx::{Pool, Sqlite};

#[derive(Clone)]
pub struct AppState {
    pub pool: Pool<Sqlite>,
    pub config: Config,
}

#[derive(Debug, Clone, sqlx::FromRow, serde::Serialize, serde::Deserialize)]
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

#[derive(Debug, Clone, sqlx::FromRow, serde::Serialize, serde::Deserialize)]
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

#[derive(serde::Deserialize)]
pub struct CreateArticleForm {
    pub title: String,
    pub content: String,
    pub cover_image: Option<String>,
    #[serde(default)]
    pub tag_ids: Vec<i64>,
}

#[derive(serde::Deserialize)]
pub struct CreateTagForm {
    pub name: String,
}

#[derive(serde::Deserialize)]
pub struct SearchQuery {
    pub q: Option<String>,
}
