//! Data types shared across Bloogla: posts, tags, media, people.

use crate::i18n::Lang;

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, sqlx::FromRow, Serialize)]
pub struct Post {
    pub id: i64,
    pub title: String,
    pub slug: String,
    pub content: String,
    pub cover_image: Option<String>,
    /// Size of the cover image in pixels, when it's in the media library.
    pub cover_width: Option<i64>,
    pub cover_height: Option<i64>,
    /// A smaller copy of the cover (800px wide), when there is one.
    pub cover_small: Option<String>,
    pub views: i64,
    pub created_at: String,
    pub status: String,
    pub published_at: String,
    pub is_page: bool,
    pub author_id: Option<i64>,
    /// Display name of the author, when they've set one.
    pub author_name: Option<String>,
    #[sqlx(skip)]
    pub reading_time: u32,
    #[sqlx(skip)]
    pub tags: Vec<Tag>,
    /// Public address (`/post/slug`, or `/slug` for a page), filled in for themes.
    #[sqlx(skip)]
    pub url: String,
}

#[derive(Debug, Clone, sqlx::FromRow, Serialize)]
pub struct Tag {
    pub id: i64,
    pub name: String,
    pub slug: String,
}

/// What a signed-in person may do.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Role {
    /// Everything, including settings and users.
    Admin,
    /// All posts, pages, media and tags.
    Editor,
    /// Their own posts.
    Author,
}

impl Role {
    pub const ALL: [Role; 3] = [Role::Admin, Role::Editor, Role::Author];

    pub fn parse(value: &str) -> Option<Self> {
        match value {
            "admin" => Some(Role::Admin),
            "editor" => Some(Role::Editor),
            "author" => Some(Role::Author),
            _ => None,
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Role::Admin => "admin",
            Role::Editor => "editor",
            Role::Author => "author",
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Role::Admin => "Admin",
            Role::Editor => "Editor",
            Role::Author => "Author",
        }
    }

    pub fn description(self) -> &'static str {
        match self {
            Role::Admin => "Everything, including settings and people",
            Role::Editor => "All posts, pages, media and tags",
            Role::Author => "Writes and publishes their own posts",
        }
    }
}

/// Session key holding the signed-in user's id.
pub const SESSION_USER_ID: &str = "user_id";

/// Session key holding the account's `session_version` at sign-in. When the
/// account's number goes up (password changed), older sessions stop working.
pub const SESSION_VERSION: &str = "session_version";

/// The signed-in person, loaded on every admin request.
#[derive(Debug, Clone)]
pub struct CurrentUser {
    pub id: i64,
    pub name: String,
    pub email: String,
    pub role: Role,
    /// Admin panel language (their own choice, or the site's).
    pub lang: Lang,
    /// See [`SESSION_VERSION`].
    pub session_version: i64,
    /// Numbers next to menu items. Filled in for page loads only.
    pub badges: Badges,
}

/// Small numbers next to menu items, like WordPress's bubbles, telling
/// someone there's something new to look at.
#[derive(Debug, Clone, Default)]
pub struct Badges {
    /// Comments waiting for approval (shown to admins and editors).
    pub pending_comments: i64,
    /// A newer Bloogla version is out (shown to admins).
    pub update_available: bool,
}

impl Badges {
    /// What `user` should see right now.
    pub async fn load(state: &crate::app::state::AppState, user: &CurrentUser) -> Self {
        let pending_comments = if user.can_edit_all() {
            crate::db::or_log(
                sqlx::query_scalar("SELECT COUNT(*) FROM comments WHERE status = 'pending'")
                    .fetch_one(&state.pool)
                    .await,
                "count waiting comments",
            )
        } else {
            0
        };
        Self {
            pending_comments,
            update_available: user.is_admin()
                && crate::services::updates::available(&state.newer_release).is_some(),
        }
    }

    /// The number as shown: big numbers stop at "99+".
    pub fn comments_label(&self) -> String {
        match self.pending_comments {
            n if n > 99 => "99+".to_string(),
            n => n.to_string(),
        }
    }
}

impl CurrentUser {
    /// Translate interface text into this person's language.
    pub fn t(&self, key: &'static str) -> &'static str {
        self.lang.t(key)
    }

    /// "1 post" / "3 posts" in this person's language.
    pub fn count(&self, n: &i64, one: &'static str, many: &'static str) -> String {
        self.lang.count(*n, one, many)
    }

    /// Translate with one `{placeholder}` filled in.
    pub fn tv(&self, key: &'static str, value: impl std::fmt::Display) -> String {
        self.lang.tv(key, value)
    }

    /// Translate with three `{placeholders}` filled in, in order.
    pub fn tv3(
        &self,
        key: &'static str,
        a: impl std::fmt::Display,
        b: impl std::fmt::Display,
        c: impl std::fmt::Display,
    ) -> String {
        self.lang.tv3(key, a, b, c)
    }

    /// An inline link with translated text, for filling into sentences.
    pub fn link(&self, href: &str, text: &'static str) -> String {
        format!(
            r#"<a href="{href}" style="color: inherit">{}</a>"#,
            self.t(text)
        )
    }

    /// Translate with two `{placeholders}` filled in, in order.
    pub fn tv2(
        &self,
        key: &'static str,
        a: impl std::fmt::Display,
        b: impl std::fmt::Display,
    ) -> String {
        self.lang.tv2(key, a, b)
    }

    pub fn is_admin(&self) -> bool {
        self.role == Role::Admin
    }

    /// Admins and editors manage everyone's content.
    pub fn can_edit_all(&self) -> bool {
        matches!(self.role, Role::Admin | Role::Editor)
    }

    /// Whether this person may change a post written by `author_id`.
    pub fn can_edit(&self, author_id: Option<i64>, is_page: bool) -> bool {
        self.can_edit_all() || (!is_page && author_id == Some(self.id))
    }

    /// Name to show; falls back to the part of the email before `@`.
    pub fn display_name(&self) -> &str {
        if self.name.is_empty() {
            self.email.split('@').next().unwrap_or(&self.email)
        } else {
            &self.name
        }
    }
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
    /// `draft`, `published` or `scheduled`.
    pub status: Option<String>,
    /// UTC date and time, e.g. `2026-10-01T12:00`.
    pub published_at: Option<String>,
    #[serde(default)]
    pub tag_ids: Vec<i64>,
    /// Names of tags typed in the editor; created when they don't exist yet.
    #[serde(default)]
    pub new_tags: Vec<String>,
}

impl PostForm {
    /// The submitted values as a `Post`, with status and date normalized.
    /// The publish date was typed in the site's time zone `tz`.
    pub fn to_post(
        &self,
        id: i64,
        is_page: bool,
        author_id: Option<i64>,
        slug: String,
        tz: chrono_tz::Tz,
    ) -> Post {
        // Only known statuses are stored; anything else publishes.
        let status = match self.status.as_deref().map(str::trim) {
            Some("draft") => "draft",
            Some("scheduled") => "scheduled",
            _ => "published",
        };
        Post {
            id,
            title: self.title.clone(),
            slug,
            content: self.content.clone(),
            cover_image: self.cover_image.clone().filter(|c| !c.trim().is_empty()),
            cover_width: None,
            cover_height: None,
            cover_small: None,
            views: 0,
            created_at: String::new(),
            status: status.to_string(),
            published_at: crate::content::text::normalize_datetime(
                self.published_at.as_deref(),
                tz,
            ),
            is_page,
            author_id,
            author_name: None,
            reading_time: 0,
            tags: Vec::new(),
            url: String::new(),
        }
    }
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

/// An uploaded image in the media library.
#[derive(Debug, Clone, sqlx::FromRow)]
pub struct Media {
    pub id: i64,
    pub filename: String,
    /// The 800px-wide copy, for wide photos.
    pub small_filename: Option<String>,
    pub original_name: String,
    /// What the picture shows, for people who can't see it.
    pub alt_text: String,
    pub size_bytes: i64,
    pub width: Option<i64>,
    pub height: Option<i64>,
}
