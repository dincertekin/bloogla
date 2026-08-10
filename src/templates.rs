use crate::models::{Post, Tag};
use askama::Template;

/// Homepage template (`templates/index.html`)
#[derive(Template)]
#[template(path = "index.html")]
pub struct IndexTemplate {
    pub blog_name: String,
    pub posts: Vec<Post>,
}

/// Admin login page template (`templates/login.html`)
#[derive(Template)]
#[template(path = "login.html")]
pub struct LoginTemplate {
    pub error: Option<String>,
}

/// New post card template (`templates/post_item.html`)
#[derive(Template)]
#[template(path = "post_item.html")]
pub struct PostItemTemplate {
    pub post: Post,
}

/// Post edit page template (`templates/edit_post.html`)
#[derive(Template)]
#[template(path = "edit_post.html")]
pub struct EditPostTemplate {
    pub blog_name: String,
    pub active_page: &'static str,
    pub post: Post,
    pub error: Option<String>,
    pub tag_checkboxes: Vec<(Tag, bool)>,
}

/// Post details page template (`templates/post.html`)
#[derive(Template)]
#[template(path = "post.html")]
pub struct PostTemplate {
    pub blog_name: String,
    pub content_html: String,
    pub post: Post,
}

/// Admin dashboard template (`templates/admin.html`)
#[derive(Template)]
#[template(path = "admin.html")]
pub struct AdminTemplate {
    pub blog_name: String,
    pub active_page: &'static str,
    pub total_posts: i64,
    pub total_views: i64,
    pub posts: Vec<Post>,
    pub top_posts: Vec<Post>,
    pub max_views: i64,
    pub growth_points: String,
    pub growth_max: i64,
    pub growth_first_date: String,
    pub growth_last_date: String,
}

/// Admin all posts page template (`templates/posts.html`)
#[derive(Template)]
#[template(path = "posts.html")]
pub struct PostsTemplate {
    pub blog_name: String,
    pub active_page: &'static str,
    pub posts: Vec<Post>,
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
    pub posts: Vec<Post>,
}
