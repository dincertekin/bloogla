use crate::models::{Media, Post, Tag};
use askama_axum::Template;

/// Admin login page template.
#[derive(Template)]
#[template(path = "admin/login.html")]
pub struct LoginTemplate {
    pub error: Option<String>,
}

/// Admin dashboard template.
#[derive(Template)]
#[template(path = "admin/dashboard.html")]
pub struct AdminTemplate {
    pub blog_name: String,
    pub welcome: bool,
    pub total_posts: i64,
    pub total_drafts: i64,
    pub total_views: i64,
    pub top_posts: Vec<Post>,
    pub max_views: i64,
    /// Views in the last 30 days, and the chart line for them.
    pub views_30d: i64,
    pub views_points: String,
    pub chart_start: String,
    /// Sites that sent visitors in the last 30 days.
    pub referrers: Vec<(String, i64)>,
    pub max_referrer: i64,
}

/// A status tab above the post list, e.g. "Drafts (2)".
pub struct StatusFilter {
    pub value: &'static str,
    pub label: &'static str,
    pub count: usize,
}

impl StatusFilter {
    pub fn new(value: &'static str, label: &'static str, count: usize) -> Self {
        Self {
            value,
            label,
            count,
        }
    }
}

/// One row of the post or page list.
pub struct PostRow {
    pub post: Post,
    pub date: String,
    /// Public URL path when the item is visible to visitors.
    pub public_path: Option<String>,
}

/// Admin post or page list.
#[derive(Template)]
#[template(path = "admin/posts.html")]
pub struct PostsTemplate {
    pub blog_name: String,
    pub is_page: bool,
    pub admin_path: &'static str,
    pub rows: Vec<PostRow>,
    pub filters: Vec<StatusFilter>,
    pub active_filter: String,
}

/// Full-page editor for posts and pages (`post.id == 0` when new).
#[derive(Template)]
#[template(path = "admin/post_editor.html")]
pub struct PostEditorTemplate {
    pub blog_name: String,
    pub base_url: String,
    pub admin_path: &'static str,
    pub public_path: Option<String>,
    pub post: Post,
    pub published_at_input: String,
    pub tag_checkboxes: Vec<(Tag, bool)>,
    pub revisions: Vec<RevisionRow>,
    pub error: Option<String>,
    pub saved: bool,
}

/// An earlier saved version listed in the editor.
pub struct RevisionRow {
    pub id: i64,
    pub label: String,
}

/// A tag with the number of posts using it.
pub struct TagRow {
    pub tag: Tag,
    pub post_count: i64,
}

/// Admin tags page template.
#[derive(Template)]
#[template(path = "admin/tags.html")]
pub struct TagsTemplate {
    pub blog_name: String,
    pub tags: Vec<TagRow>,
}

/// New tag row partial template.
#[derive(Template)]
#[template(path = "admin/tag_item.html")]
pub struct TagItemTemplate {
    pub row: TagRow,
}

#[derive(Template)]
#[template(path = "admin/settings.html")]
pub struct SettingsTemplate {
    pub blog_name: String,
    pub blog_description: String,
    pub blog_keywords: String,
    pub posts_per_page: String,
    pub nav_menu: String,
    pub show_views: bool,
    pub publisher_name: String,
    pub publisher_type: String,
    pub site_icon: String,
    pub active_theme: String,
    pub available_themes: Vec<crate::themes::ThemeInfo>,
}

/// Media library page template.
#[derive(Template)]
#[template(path = "admin/media.html")]
pub struct MediaTemplate {
    pub blog_name: String,
    pub media: Vec<MediaView>,
    pub error: Option<String>,
}

/// A media library entry prepared for display.
pub struct MediaView {
    pub id: i64,
    pub url: String,
    pub original_name: String,
    /// Alt text guessed from the file name.
    pub alt: String,
    pub markdown: String,
    pub size_label: String,
    pub dimensions: String,
}

impl From<Media> for MediaView {
    fn from(m: Media) -> Self {
        let url = format!("/uploads/{}", m.filename);
        let alt = m
            .original_name
            .rsplit_once('.')
            .map_or(m.original_name.as_str(), |(stem, _)| stem)
            .replace(['[', ']'], "");
        let size_label = if m.size_bytes >= 1024 * 1024 {
            format!("{:.1} MB", m.size_bytes as f64 / (1024.0 * 1024.0))
        } else {
            format!("{} KB", (m.size_bytes + 1023) / 1024)
        };
        let dimensions = match (m.width, m.height) {
            (Some(w), Some(h)) => format!("{w}×{h}"),
            _ => String::new(),
        };
        Self {
            id: m.id,
            markdown: format!("![{alt}]({url})"),
            alt,
            url,
            original_name: m.original_name,
            size_label,
            dimensions,
        }
    }
}

/// Media chooser shown inside the post editor.
#[derive(Template)]
#[template(path = "admin/media_picker.html")]
pub struct MediaPickerTemplate {
    pub media: Vec<MediaView>,
    pub error: Option<String>,
}

/// First-run browser setup page template.
#[derive(Template)]
#[template(path = "admin/setup.html")]
pub struct SetupTemplate {
    pub blog_name: String,
    pub email: String,
    pub error: Option<String>,
}
