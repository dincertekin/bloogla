use crate::models::{Article, Tag};
use askama_axum::Template;

/// Public homepage template.
#[derive(Template)]
#[template(path = "public/index.html")]
pub struct IndexTemplate {
    pub blog_name: String,
    pub articles: Vec<Article>,
    pub tags: Vec<Tag>,
    pub search_query: String,
}

/// Admin login page template.
#[derive(Template)]
#[template(path = "admin/login.html")]
pub struct LoginTemplate {
    pub error: Option<String>,
}

/// New article card partial template.
#[derive(Template)]
#[template(path = "partials/article_item.html")]
pub struct ArticleItemTemplate {
    pub article: Article,
}

/// Admin article edit page template.
#[derive(Template)]
#[template(path = "admin/edit_article.html")]
pub struct EditArticleTemplate {
    pub blog_name: String,
    pub article: Article,
    pub error: Option<String>,
    pub tag_checkboxes: Vec<(Tag, bool)>,
}

/// Public article details page template.
#[derive(Template)]
#[template(path = "public/article.html")]
pub struct ArticleTemplate {
    pub blog_name: String,
    pub content_html: String,
    pub article: Article,
    pub tags: Vec<Tag>,
    pub search_query: String,
}

/// Admin dashboard template.
#[derive(Template)]
#[template(path = "admin/dashboard.html")]
pub struct AdminTemplate {
    pub blog_name: String,
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

/// Admin articles page template.
#[derive(Template)]
#[template(path = "admin/articles.html")]
pub struct ArticlesTemplate {
    pub blog_name: String,
    pub articles: Vec<Article>,
    pub all_tags: Vec<Tag>,
}

/// Admin tags page template.
#[derive(Template)]
#[template(path = "admin/tags.html")]
pub struct TagsTemplate {
    pub blog_name: String,
    pub tags: Vec<Tag>,
}

/// Public tag page template.
#[derive(Template)]
#[template(path = "public/tag.html")]
pub struct TagPageTemplate {
    pub blog_name: String,
    pub tag_name: String,
    pub articles: Vec<Article>,
    pub tags: Vec<Tag>,
    pub search_query: String,
}

/// New tag item partial template.
#[derive(Template)]
#[template(path = "partials/tag_item.html")]
pub struct TagItemTemplate {
    pub tag: Tag,
}

#[derive(Template)]
#[template(path = "admin/settings.html")]
pub struct SettingsTemplate {
    pub blog_name: String,
    pub blog_description: String,
    pub articles_per_page: String,
}
