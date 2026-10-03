//! Admin screens for posts and pages: list, editor, preview and delete.
//! Saving itself happens in `db/posts.rs`, shared with the JSON API.

use super::{forbidden, StatusFilter};
use crate::app::models::{CurrentUser, Post, PostForm, Tag};
use crate::app::state::AppState;
use crate::content::text::{display_date, display_datetime, to_datetime_local, url_encode};
use crate::db::posts::{self, SaveError, POST_SELECT};
use crate::db::{or_log, settings, tags};
use crate::i18n::Lang;
use crate::server::routes::post_path;

use askama::Template;
use axum::extract::{Extension, Path, Query, State};
use axum::http::StatusCode;
use axum::response::{Html, IntoResponse, Redirect, Response};
use axum_extra::extract::Form;
use serde::Deserialize;
use std::collections::HashSet;

/// One row of the post or page list.
pub struct PostRow {
    pub post: Post,
    pub date: String,
    /// Public URL path when the item is visible to visitors.
    pub public_path: Option<String>,
}

/// Admin post or page list.
#[derive(Template)]
#[template(path = "posts.html")]
pub struct PostsTemplate {
    pub blog_name: String,
    pub me: CurrentUser,
    pub is_page: bool,
    pub admin_path: &'static str,
    pub rows: Vec<PostRow>,
    pub filters: Vec<StatusFilter>,
    pub active_filter: String,
}

/// Full-page editor for posts and pages (`post.id == 0` when new).
#[derive(Template)]
#[template(path = "post_editor.html")]
pub struct PostEditorTemplate {
    pub blog_name: String,
    pub me: CurrentUser,
    pub base_url: String,
    pub admin_path: &'static str,
    pub public_path: Option<String>,
    pub post: Post,
    pub published_at_input: String,
    pub tag_checkboxes: Vec<(Tag, bool)>,
    pub revisions: Vec<RevisionRow>,
    /// Newsletter: `None` when it can't be used for this post.
    pub newsletter: Option<NewsletterStatus>,
    /// Result of emailing the post: (worked, message).
    pub email_notice: Option<(bool, String)>,
    pub error: Option<String>,
    pub saved: bool,
}

/// An earlier saved version listed in the editor.
pub struct RevisionRow {
    pub id: i64,
    pub label: String,
}

/// Whether a post can be, or has been, emailed to subscribers.
pub struct NewsletterStatus {
    pub subscribers: i64,
    /// When it was emailed, and to how many people.
    pub sent: Option<(String, i64)>,
}

/// Whether an admin screen works with blog posts or standalone pages.
#[derive(Clone, Copy, PartialEq)]
pub enum Kind {
    Post,
    Page,
}

impl Kind {
    fn is_page(self) -> bool {
        self == Kind::Page
    }

    fn of(post: &Post) -> Self {
        if post.is_page {
            Kind::Page
        } else {
            Kind::Post
        }
    }

    /// Admin URL prefix, e.g. `/admin/posts`.
    pub fn admin_path(self) -> &'static str {
        match self {
            Kind::Post => "/admin/posts",
            Kind::Page => "/admin/pages",
        }
    }
}

#[derive(Deserialize)]
pub struct ListQuery {
    status: Option<String>,
}

#[derive(Deserialize)]
pub struct EditQuery {
    saved: Option<String>,
    /// Number of subscribers the post is being emailed to.
    emailed: Option<i64>,
    /// Why emailing the post failed.
    email_error: Option<String>,
}

#[derive(Deserialize)]
pub struct PreviewForm {
    content: String,
}

pub async fn list_posts(
    state: State<AppState>,
    me: Extension<CurrentUser>,
    query: Query<ListQuery>,
) -> Response {
    list(state, me, query, Kind::Post).await
}

pub async fn list_pages(
    state: State<AppState>,
    me: Extension<CurrentUser>,
    query: Query<ListQuery>,
) -> Response {
    list(state, me, query, Kind::Page).await
}

/// GET /admin/posts, /admin/pages -> List with status filters.
/// Authors only see their own posts.
async fn list(
    State(state): State<AppState>,
    Extension(me): Extension<CurrentUser>,
    Query(query): Query<ListQuery>,
    kind: Kind,
) -> Response {
    let mut posts = or_log(
        sqlx::query_as::<_, Post>(&format!(
            "{POST_SELECT} WHERE is_page = ? AND (? OR author_id = ?)
             ORDER BY published_at DESC, id DESC"
        ))
        .bind(kind.is_page())
        .bind(me.can_edit_all())
        .bind(me.id)
        .fetch_all(&state.pool)
        .await,
        "list posts",
    );

    let count = |status: &str| posts.iter().filter(|p| p.status == status).count();
    let filters = vec![
        StatusFilter::new("", "All", posts.len()),
        StatusFilter::new("published", "Published", count("published")),
        StatusFilter::new("scheduled", "Scheduled", count("scheduled")),
        StatusFilter::new("draft", "Drafts", count("draft")),
    ];

    let active = query.status.unwrap_or_default();
    if !active.is_empty() {
        posts.retain(|p| p.status == active);
    }

    let ids: Vec<i64> = posts.iter().map(|p| p.id).collect();
    let mut tag_map = tags::for_posts(&state.pool, &ids).await;

    let rows = posts
        .into_iter()
        .map(|mut post| {
            post.tags = tag_map.remove(&post.id).unwrap_or_default();
            PostRow {
                date: display_date(me.lang, &post.published_at),
                public_path: posts::is_live(&post).then(|| post_path(&post.slug, post.is_page)),
                post,
            }
        })
        .collect();

    PostsTemplate {
        blog_name: settings::load(&state.pool).await.blog_name.clone(),
        me,
        is_page: kind.is_page(),
        admin_path: kind.admin_path(),
        rows,
        filters,
        active_filter: active,
    }
    .into_response()
}

pub async fn new_post(state: State<AppState>, me: Extension<CurrentUser>) -> Response {
    new_page_for(state, me, Kind::Post).await
}

pub async fn new_page(state: State<AppState>, me: Extension<CurrentUser>) -> Response {
    new_page_for(state, me, Kind::Page).await
}

/// GET /admin/posts/new, /admin/pages/new -> Empty editor.
async fn new_page_for(
    State(state): State<AppState>,
    Extension(me): Extension<CurrentUser>,
    kind: Kind,
) -> Response {
    let post = Post {
        id: 0,
        title: String::new(),
        slug: String::new(),
        content: String::new(),
        cover_image: None,
        cover_width: None,
        cover_height: None,
        views: 0,
        created_at: String::new(),
        status: "draft".to_string(),
        published_at: String::new(),
        is_page: kind.is_page(),
        author_id: Some(me.id),
        author_name: Some(me.display_name().to_string()),
        reading_time: 0,
        tags: Vec::new(),
        fields: Default::default(),
    };
    editor(&state, me, post, &HashSet::new(), None, false, None).await
}

/// What the editor's newsletter option should show, if anything.
async fn newsletter_status(
    state: &AppState,
    me: &CurrentUser,
    post: &Post,
) -> Option<NewsletterStatus> {
    if post.is_page
        || !me.can_edit_all()
        || !settings::load(&state.pool).await.newsletter
        || crate::services::email::smtp_settings(state).await.is_none()
    {
        return None;
    }
    let subscribers: i64 = or_log(
        sqlx::query_scalar("SELECT COUNT(*) FROM subscribers WHERE status = 'active'")
            .fetch_one(&state.pool)
            .await,
        "count subscribers",
    );
    let sent: Option<(String, i64)> = or_log(
        sqlx::query_as("SELECT started_at, recipients FROM newsletter_sends WHERE post_id = ?")
            .bind(post.id)
            .fetch_optional(&state.pool)
            .await,
        "newsletter sent",
    );
    Some(NewsletterStatus {
        subscribers,
        sent: sent.map(|(at, n)| (display_date(me.lang, &at), n)),
    })
}

/// Render the editor for `post` with `selected_tags` checked.
async fn editor(
    state: &AppState,
    me: CurrentUser,
    post: Post,
    selected_tags: &HashSet<i64>,
    error: Option<String>,
    saved: bool,
    email_notice: Option<(bool, String)>,
) -> Response {
    let tag_checkboxes = tags::all(&state.pool)
        .await
        .into_iter()
        .map(|t| {
            let checked = selected_tags.contains(&t.id);
            (t, checked)
        })
        .collect();

    let kind = Kind::of(&post);
    let revisions: Vec<(i64, String)> = if post.id == 0 {
        Vec::new()
    } else {
        or_log(
            sqlx::query_as(
                "SELECT id, saved_at FROM post_revisions WHERE post_id = ?
                 ORDER BY id DESC LIMIT 10",
            )
            .bind(post.id)
            .fetch_all(&state.pool)
            .await,
            "list revisions",
        )
    };
    let revisions = revisions
        .into_iter()
        .map(|(id, saved_at)| RevisionRow {
            id,
            label: display_datetime(me.lang, &saved_at),
        })
        .collect();
    let newsletter = newsletter_status(state, &me, &post).await;
    let status = if error.is_some() {
        StatusCode::UNPROCESSABLE_ENTITY
    } else {
        StatusCode::OK
    };

    (
        status,
        PostEditorTemplate {
            blog_name: settings::load(&state.pool).await.blog_name.clone(),
            me,
            base_url: state.config.base_url.clone(),
            admin_path: kind.admin_path(),
            public_path: (post.id != 0 && posts::is_live(&post))
                .then(|| post_path(&post.slug, post.is_page)),
            published_at_input: to_datetime_local(&post.published_at),
            post,
            tag_checkboxes,
            revisions,
            newsletter,
            email_notice,
            error,
            saved,
        },
    )
        .into_response()
}

/// GET /admin/posts/:id/edit, /admin/pages/:id/edit -> Editor for an existing item.
pub async fn edit(
    State(state): State<AppState>,
    Extension(me): Extension<CurrentUser>,
    Path(id): Path<i64>,
    Query(query): Query<EditQuery>,
) -> Response {
    let Some(mut post) = posts::find(&state.pool, id).await else {
        return (StatusCode::NOT_FOUND, Html(me.t("Post not found"))).into_response();
    };
    if !me.can_edit(post.author_id, post.is_page) {
        return forbidden(me.lang);
    }
    posts::load_tags_and_fields(&state.pool, std::slice::from_mut(&mut post)).await;
    let selected: HashSet<i64> = post.tags.iter().map(|t| t.id).collect();
    let email_notice = match (query.emailed, query.email_error) {
        (Some(count), _) => Some((
            true,
            me.count(
                &count,
                "Emailing it to {n} subscriber.",
                "Emailing it to {n} subscribers.",
            ),
        )),
        (None, Some(error)) => Some((
            false,
            me.tv("Saved, but not emailed: {error}", me.lang.t_owned(&error)),
        )),
        _ => None,
    };
    editor(
        &state,
        me,
        post,
        &selected,
        None,
        query.saved.is_some(),
        email_notice,
    )
    .await
}

pub async fn create_post(
    state: State<AppState>,
    me: Extension<CurrentUser>,
    form: Form<PostForm>,
) -> Response {
    create(state, me, form, Kind::Post).await
}

pub async fn create_page(
    state: State<AppState>,
    me: Extension<CurrentUser>,
    form: Form<PostForm>,
) -> Response {
    create(state, me, form, Kind::Page).await
}

/// Re-show the editor with the submitted values and a message.
async fn editor_with_error(
    state: &AppState,
    me: CurrentUser,
    id: i64,
    is_page: bool,
    author_id: Option<i64>,
    form: &PostForm,
) -> Response {
    let post = form.to_post(
        id,
        is_page,
        author_id,
        form.slug.clone().unwrap_or_default(),
    );
    let tags = form.tag_ids.iter().copied().collect();
    let message = me.t("Add a title first.").to_string();
    editor(state, me, post, &tags, Some(message), false, None).await
}

/// After saving, email the post to subscribers if the editor asked to.
/// Returns what to add to the editor URL so the result is shown.
async fn email_if_requested(
    state: &AppState,
    me: &CurrentUser,
    id: i64,
    form: &PostForm,
) -> String {
    if form.send_newsletter.is_none() || !me.can_edit_all() {
        return String::new();
    }
    match crate::services::email::send_post_to_subscribers(state, id).await {
        Ok(count) => format!("&emailed={count}"),
        Err(e) => format!("&email_error={}", url_encode(&e)),
    }
}

/// POST /admin/posts, /admin/pages -> Create, then continue in the editor.
async fn create(
    State(state): State<AppState>,
    Extension(me): Extension<CurrentUser>,
    Form(form): Form<PostForm>,
    kind: Kind,
) -> Response {
    match posts::create(&state.pool, &me, kind.is_page(), &form).await {
        Ok(id) => {
            crate::services::webmention::send_for_post(&state, id);
            let emailed = email_if_requested(&state, &me, id, &form).await;
            Redirect::to(&format!("{}/{id}/edit?saved=1{emailed}", kind.admin_path()))
                .into_response()
        }
        Err(SaveError::MissingTitle) => {
            let author = Some(me.id);
            editor_with_error(&state, me, 0, kind.is_page(), author, &form).await
        }
        Err(SaveError::Forbidden) => forbidden(me.lang),
        Err(_) => database_error(me.lang),
    }
}

/// POST /admin/posts/:id/edit -> Save changes and stay in the editor.
pub async fn update(
    State(state): State<AppState>,
    Extension(me): Extension<CurrentUser>,
    Path(id): Path<i64>,
    Form(form): Form<PostForm>,
) -> Response {
    match posts::update(&state.pool, &me, id, &form).await {
        Ok(()) => {
            crate::services::webmention::send_for_post(&state, id);
            let is_page = posts::owner(&state.pool, id).await.is_some_and(|(_, p)| p);
            let kind = if is_page { Kind::Page } else { Kind::Post };
            let emailed = email_if_requested(&state, &me, id, &form).await;
            Redirect::to(&format!("{}/{id}/edit?saved=1{emailed}", kind.admin_path()))
                .into_response()
        }
        Err(SaveError::MissingTitle) => {
            let (author_id, is_page) = posts::owner(&state.pool, id).await.unwrap_or((None, false));
            editor_with_error(&state, me, id, is_page, author_id, &form).await
        }
        Err(SaveError::NotFound) => {
            (StatusCode::NOT_FOUND, Html(me.t("Post not found"))).into_response()
        }
        Err(SaveError::Forbidden) => forbidden(me.lang),
        Err(SaveError::Database) => database_error(me.lang),
    }
}

/// GET /admin/posts/:id/revisions/:rev -> One earlier version, for restoring in the editor.
pub async fn revision(
    State(state): State<AppState>,
    Extension(me): Extension<CurrentUser>,
    Path((id, rev)): Path<(i64, i64)>,
) -> Response {
    if !posts::may_edit(&state.pool, &me, id).await {
        return forbidden(me.lang);
    }
    let row: Option<(String, String)> = or_log(
        sqlx::query_as("SELECT title, content FROM post_revisions WHERE id = ? AND post_id = ?")
            .bind(rev)
            .bind(id)
            .fetch_optional(&state.pool)
            .await,
        "load revision",
    );
    match row {
        Some((title, content)) => {
            axum::Json(serde_json::json!({ "title": title, "content": content })).into_response()
        }
        None => StatusCode::NOT_FOUND.into_response(),
    }
}

/// DELETE /admin/posts/:id -> Remove a post or page.
pub async fn delete(
    State(state): State<AppState>,
    Extension(me): Extension<CurrentUser>,
    Path(id): Path<i64>,
) -> StatusCode {
    match posts::delete(&state.pool, &me, id).await {
        Ok(()) => StatusCode::OK,
        Err(SaveError::NotFound) => StatusCode::NOT_FOUND,
        Err(SaveError::Forbidden) => StatusCode::FORBIDDEN,
        Err(_) => StatusCode::INTERNAL_SERVER_ERROR,
    }
}

/// POST /admin/preview -> Rendered Markdown for the editor's Preview tab.
pub async fn preview(
    State(state): State<AppState>,
    Extension(me): Extension<CurrentUser>,
    Form(form): Form<PreviewForm>,
) -> Html<String> {
    if form.content.trim().is_empty() {
        return Html(format!(
            r#"<p class="text-muted">{}</p>"#,
            me.t("Nothing to preview yet.")
        ));
    }
    Html(crate::content::render(&state, &form.content).await)
}

fn database_error(lang: Lang) -> Response {
    (
        StatusCode::INTERNAL_SERVER_ERROR,
        Html(lang.t("Something went wrong while saving. Please try again.")),
    )
        .into_response()
}
