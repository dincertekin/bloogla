use crate::models::{Article, Tag};
use askama::Template;

/// Homepage template (`templates/index.html`)
#[derive(Template)]
#[template(path = "index.html")]
pub struct IndexTemplate {
    pub blog_name: String,
    pub articles: Vec<Article>,
    pub tags: Vec<Tag>,
    pub search_query: String,
}

/// Admin login page template (`templates/login.html`)
#[derive(Template)]
#[template(path = "login.html")]
pub struct LoginTemplate {
    pub error: Option<String>,
}

/// New article card template (`templates/article_item.html`)
#[derive(Template)]
#[template(path = "article_item.html")]
pub struct ArticleItemTemplate {
    pub article: Article,
}

/// Article edit page template (`templates/edit_article.html`)
#[derive(Template)]
#[template(path = "edit_article.html")]
pub struct EditArticleTemplate {
    pub blog_name: String,
    pub active_page: &'static str,
    pub article: Article,
    pub error: Option<String>,
    pub tag_checkboxes: Vec<(Tag, bool)>,
}

/// Article details page template (`templates/article.html`)
#[derive(Template)]
#[template(path = "article.html")]
pub struct ArticleTemplate {
    pub blog_name: String,
    pub content_html: String,
    pub article: Article,
    pub tags: Vec<Tag>,
    pub search_query: String,
}

/// Admin dashboard template (`templates/admin.html`)
#[derive(Template)]
#[template(path = "admin.html")]
pub struct AdminTemplate {
    pub blog_name: String,
    pub active_page: &'static str,
    pub total_articles: i64,
    pub total_views: i64,
    pub articles: Vec<Article>,
    pub top_articles: Vec<Article>,
    pub max_views: i64,
    pub growth_points: String,
    pub growth_max: i64,
    pub growth_first_date: String,
    pub growth_last_date: String,
}

/// Admin all articles page template (`templates/articles.html`)
#[derive(Template)]
#[template(path = "articles.html")]
pub struct ArticlesTemplate {
    pub blog_name: String,
    pub active_page: &'static str,
    pub articles: Vec<Article>,
    pub all_tags: Vec<Tag>,
}

/// Admin tags page template (`templates/tags.html`)
#[derive(Template)]
#[template(path = "tags.html")]
pub struct TagsTemplate {
    pub blog_name: String,
    pub active_page: &'static str,
    pub tags: Vec<Tag>,
}

/// Public tag page template (`templates/tag.html`)
#[derive(Template)]
#[template(path = "tag.html")]
pub struct TagPageTemplate {
    pub blog_name: String,
    pub tag_name: String,
    pub articles: Vec<Article>,
    pub tags: Vec<Tag>,
    pub search_query: String,
}

/// New tag item template (`templates/tag_item.html`)
#[derive(Template)]
#[template(path = "tag_item.html")]
pub struct TagItemTemplate {
    pub tag: Tag,
}
