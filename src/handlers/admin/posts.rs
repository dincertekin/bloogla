//! Admin screens for posts and pages: list, editor, preview and delete.
//! Saving itself happens in `db/posts.rs`.

use super::{forbidden, StatusFilter};
use crate::app::models::{CurrentUser, Post, PostForm, Tag};
use crate::app::state::AppState;
use crate::content::text::{display_date, display_datetime, escape_html, to_datetime_local};
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
    /// In the trash: how many days ago it was moved there.
    pub days_in_trash: Option<i64>,
}

impl PostRow {
    /// Days since it went to the trash (0 when it isn't there).
    pub fn trash_days(&self) -> i64 {
        self.days_in_trash.unwrap_or(0)
    }
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
    /// Showing the trash (`?status=trash`).
    pub in_trash: bool,
    /// How many items are in the trash (for "Empty trash").
    pub trash_count: i64,
    /// Days before trashed posts are deleted by themselves.
    pub trash_days: i64,
}

/// Full-page editor for posts and pages (`post.id == 0` when new).
#[derive(Template)]
#[template(path = "post_editor.html")]
pub struct PostEditorTemplate {
    pub blog_name: String,
    /// The site's time zone, e.g. `Europe/Istanbul` (dates are typed in it).
    pub timezone: String,
    pub me: CurrentUser,
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

    // The trash is a list of its own; every other list leaves it out.
    let trashed: Vec<Post>;
    (trashed, posts) = posts.into_iter().partition(|p| p.trashed_at.is_some());
    let active = query.status.unwrap_or_default();
    let in_trash = active == "trash";

    let count = |status: &str| posts.iter().filter(|p| p.status == status).count();
    let mut filters = vec![
        StatusFilter::new("", "All", posts.len()),
        StatusFilter::new("published", "Published", count("published")),
        StatusFilter::new("scheduled", "Scheduled", count("scheduled")),
        StatusFilter::new("draft", "Drafts", count("draft")),
    ];
    // The Trash tab shows only when something is in it (or it's open).
    if !trashed.is_empty() || in_trash {
        filters.push(StatusFilter::new("trash", "Trash", trashed.len()));
    }
    let trash_count = trashed.len() as i64;

    if in_trash {
        posts = trashed;
    } else if !active.is_empty() {
        posts.retain(|p| p.status == active);
    }

    let ids: Vec<i64> = posts.iter().map(|p| p.id).collect();
    let mut tag_map = tags::for_posts(&state.pool, &ids).await;
    let tz = settings::load(&state.pool).await.timezone;

    let rows = posts
        .into_iter()
        .map(|mut post| {
            post.tags = tag_map.remove(&post.id).unwrap_or_default();
            PostRow {
                date: display_date(me.lang, tz, &post.published_at),
                public_path: posts::is_live(&post).then(|| post_path(&post.slug, post.is_page)),
                days_in_trash: post.trashed_at.as_deref().map(days_ago),
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
        in_trash,
        trash_count,
        trash_days: posts::TRASH_DAYS,
    }
    .into_response()
}

/// Whole days since a stored date (`2026-10-16 09:30:00`).
fn days_ago(date: &str) -> i64 {
    chrono::NaiveDateTime::parse_from_str(date, "%Y-%m-%d %H:%M:%S")
        .map(|then| (chrono::Utc::now().naive_utc() - then).num_days().max(0))
        .unwrap_or(0)
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
        cover_small: None,
        views: 0,
        created_at: String::new(),
        status: "draft".to_string(),
        published_at: String::new(),
        is_page: kind.is_page(),
        author_id: Some(me.id),
        author_name: Some(me.display_name().to_string()),
        trashed_at: None,
        reading_time: 0,
        tags: Vec::new(),
        url: String::new(),
    };
    editor(&state, me, post, &HashSet::new(), None, false).await
}

/// Render the editor for `post` with `selected_tags` checked.
async fn editor(
    state: &AppState,
    me: CurrentUser,
    post: Post,
    selected_tags: &HashSet<i64>,
    error: Option<String>,
    saved: bool,
) -> Response {
    let site = settings::load(&state.pool).await;
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
            label: display_datetime(me.lang, site.timezone, &saved_at),
        })
        .collect();
    let status = if error.is_some() {
        StatusCode::UNPROCESSABLE_ENTITY
    } else {
        StatusCode::OK
    };

    (
        status,
        PostEditorTemplate {
            blog_name: site.blog_name.clone(),
            timezone: site.timezone.name().to_string(),
            me,
            base_url: state.config.base_url.clone(),
            admin_path: kind.admin_path(),
            public_path: (post.id != 0 && posts::is_live(&post))
                .then(|| post_path(&post.slug, post.is_page)),
            published_at_input: to_datetime_local(site.timezone, &post.published_at),
            post,
            tag_checkboxes,
            revisions,
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
    posts::load_tags(&state.pool, std::slice::from_mut(&mut post)).await;
    let selected: HashSet<i64> = post.tags.iter().map(|t| t.id).collect();
    editor(&state, me, post, &selected, None, query.saved.is_some()).await
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
    let tz = settings::load(&state.pool).await.timezone;
    let post = form.to_post(
        id,
        is_page,
        author_id,
        form.slug.clone().unwrap_or_default(),
        tz,
    );
    let tags = form.tag_ids.iter().copied().collect();
    let message = me.t("Add a title first.").to_string();
    editor(state, me, post, &tags, Some(message), false).await
}

/// POST /admin/posts, /admin/pages -> Create, then continue in the editor.
async fn create(
    State(state): State<AppState>,
    Extension(me): Extension<CurrentUser>,
    Form(form): Form<PostForm>,
    kind: Kind,
) -> Response {
    match posts::create(&state.pool, &me, kind.is_page(), &form).await {
        Ok(id) => Redirect::to(&format!("{}/{id}/edit?saved=1", kind.admin_path())).into_response(),
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
            let is_page = posts::owner(&state.pool, id).await.is_some_and(|(_, p)| p);
            let kind = if is_page { Kind::Page } else { Kind::Post };
            Redirect::to(&format!("{}/{id}/edit?saved=1", kind.admin_path())).into_response()
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

/// What a trash action answers: nothing to show (the row closes up), or
/// why it didn't work.
fn trash_answer(result: Result<u64, SaveError>) -> StatusCode {
    match result {
        // Nothing changed: it isn't (or is no longer) where the button
        // expects, like deleting a post that isn't in the trash.
        Ok(0) => StatusCode::NOT_FOUND,
        Ok(_) => StatusCode::OK,
        Err(SaveError::NotFound) => StatusCode::NOT_FOUND,
        Err(SaveError::Forbidden) => StatusCode::FORBIDDEN,
        Err(_) => StatusCode::INTERNAL_SERVER_ERROR,
    }
}

#[derive(Deserialize)]
pub struct TrashQuery {
    /// Sent from the editor: go back to the list afterwards.
    #[serde(default)]
    back: Option<String>,
}

/// POST /admin/posts/:id/trash -> Move to the trash. No question first: it
/// can be undone, and the answer is a note with "Undo" above the list.
pub async fn move_to_trash(
    State(state): State<AppState>,
    Extension(me): Extension<CurrentUser>,
    Path(id): Path<i64>,
    Query(query): Query<TrashQuery>,
) -> Response {
    let result = posts::move_to_trash(&state.pool, &me, &[id]).await;
    let status = trash_answer(result);
    if status != StatusCode::OK {
        return status.into_response();
    }
    let Some(post) = posts::find(&state.pool, id).await else {
        return StatusCode::OK.into_response();
    };
    let kind = Kind::of(&post);
    if query.back.is_some() {
        return ([("HX-Redirect", kind.admin_path())], ()).into_response();
    }
    // Swapped into the note above the list, beside the row closing up.
    Html(format!(
        r#"<div id="trash-notice" class="trash-notice" role="status" hx-swap-oob="true"><span>{}</span><button type="button" class="btn btn-secondary btn-sm" hx-post="{}/{id}/restore?undo=1" hx-swap="none">{}</button></div>"#,
        escape_html(&me.tv("“{title}” moved to the trash.", &post.title)),
        kind.admin_path(),
        me.t("Undo"),
    ))
    .into_response()
}

#[derive(Deserialize)]
pub struct RestoreQuery {
    /// "Undo" right after moving it: show the list again with it back.
    #[serde(default)]
    undo: Option<String>,
}

/// POST /admin/posts/:id/restore -> Take out of the trash.
pub async fn restore(
    State(state): State<AppState>,
    Extension(me): Extension<CurrentUser>,
    Path(id): Path<i64>,
    Query(query): Query<RestoreQuery>,
) -> Response {
    let status = trash_answer(posts::restore(&state.pool, &me, &[id]).await);
    if status == StatusCode::OK && query.undo.is_some() {
        return ([("HX-Refresh", "true")], ()).into_response();
    }
    status.into_response()
}

/// DELETE /admin/posts/:id -> Delete a post or page in the trash for good.
pub async fn delete(
    State(state): State<AppState>,
    Extension(me): Extension<CurrentUser>,
    Path(id): Path<i64>,
) -> StatusCode {
    trash_answer(posts::delete_forever(&state.pool, &me, &[id]).await)
}

pub async fn empty_post_trash(state: State<AppState>, me: Extension<CurrentUser>) -> StatusCode {
    empty_trash(state, me, Kind::Post).await
}

pub async fn empty_page_trash(state: State<AppState>, me: Extension<CurrentUser>) -> StatusCode {
    empty_trash(state, me, Kind::Page).await
}

/// DELETE /admin/posts/trash, /admin/pages/trash -> Empty the trash, after
/// saving a backup of the site (it can't be undone).
async fn empty_trash(
    State(state): State<AppState>,
    Extension(me): Extension<CurrentUser>,
    kind: Kind,
) -> StatusCode {
    let folder = &state.config.folder;
    let backup = folder
        .join(crate::services::backup::BACKUP_DIR)
        .join(format!(
            "before-empty-trash-{}.zip",
            chrono::Utc::now().format("%Y%m%d-%H%M%S")
        ));
    let saved = match backup.parent().map(std::fs::create_dir_all) {
        Some(Err(e)) => Err(e.to_string()),
        _ => {
            crate::services::backup::write_backup(&state.pool, &folder.join("uploads"), &backup)
                .await
        }
    };
    if let Err(e) = saved {
        tracing::error!("Trash not emptied, the backup failed: {e}");
        return StatusCode::INTERNAL_SERVER_ERROR;
    }
    trash_answer(posts::empty_trash(&state.pool, &me, kind.is_page()).await)
}

#[derive(Deserialize)]
pub struct BulkForm {
    /// Which button was clicked: publish, draft, trash, restore or delete.
    action: String,
    /// The ticked posts.
    #[serde(default)]
    ids: Vec<i64>,
}

/// POST /admin/posts/bulk, /admin/pages/bulk -> Change the ticked posts at
/// once. Only the ones this person may edit change. The page reloads to
/// show the result.
pub async fn bulk(
    State(state): State<AppState>,
    Extension(me): Extension<CurrentUser>,
    Form(form): Form<BulkForm>,
) -> Response {
    if form.ids.is_empty() {
        return super::alert(me.lang, "error", "Tick the posts to change first.");
    }
    let pool = &state.pool;
    let result = match form.action.as_str() {
        "publish" => posts::publish(pool, &me, &form.ids).await,
        "draft" => posts::make_draft(pool, &me, &form.ids).await,
        "trash" => posts::move_to_trash(pool, &me, &form.ids).await,
        "restore" => posts::restore(pool, &me, &form.ids).await,
        "delete" => posts::delete_forever(pool, &me, &form.ids).await,
        _ => return StatusCode::BAD_REQUEST.into_response(),
    };
    match result {
        Ok(_) => (
            [("HX-Trigger", "reload-page")],
            super::alert(me.lang, "success", "Done."),
        )
            .into_response(),
        Err(SaveError::Forbidden | SaveError::NotFound) => super::alert(
            me.lang,
            "error",
            "These posts can't be changed by you. Ask an admin if they need changing.",
        ),
        Err(_) => super::alert(
            me.lang,
            "error",
            "Something went wrong while saving. Please try again.",
        ),
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
