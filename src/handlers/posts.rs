//! Admin screens for posts and pages: list, editor, preview and delete.

use crate::models::{AppState, Post, PostForm, POST_SELECT};
use crate::templates::{PostEditorTemplate, PostRow, PostsTemplate, RevisionRow, StatusFilter};
use crate::utils::{
    generate_unique_slug, get_setting, normalize_published_at, normalize_status, or_log,
    render_safe_markdown, to_datetime_local,
};

use axum::extract::{Path, Query, State};
use axum::http::StatusCode;
use axum::response::{Html, IntoResponse, Redirect, Response};
use axum_extra::extract::Form;
use serde::Deserialize;
use std::collections::HashSet;

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

/// True when a post is visible to visitors right now.
fn is_live(post: &Post) -> bool {
    let now = chrono::Utc::now().format("%Y-%m-%d %H:%M:%S").to_string();
    post.status != "draft" && post.published_at.replace('T', " ") <= now
}

pub async fn list_posts(state: State<AppState>, query: Query<ListQuery>) -> Response {
    list(state, query, Kind::Post).await
}

pub async fn list_pages(state: State<AppState>, query: Query<ListQuery>) -> Response {
    list(state, query, Kind::Page).await
}

/// GET /admin/posts, /admin/pages -> List with status filters.
async fn list(
    State(state): State<AppState>,
    Query(query): Query<ListQuery>,
    kind: Kind,
) -> Response {
    let mut posts = or_log(
        sqlx::query_as::<_, Post>(&format!(
            "{POST_SELECT} WHERE is_page = ? ORDER BY published_at DESC, id DESC"
        ))
        .bind(kind.is_page())
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
    let tag_map = crate::tags::get_tags_for_posts(&state.pool, &ids).await;

    let rows = posts
        .into_iter()
        .map(|mut post| {
            post.tags = tag_map.get(&post.id).cloned().unwrap_or_default();
            PostRow {
                date: crate::utils::format_display_date(&post.published_at),
                public_path: is_live(&post)
                    .then(|| crate::handlers::public::post_path(&post.slug, post.is_page)),
                post,
            }
        })
        .collect();

    PostsTemplate {
        blog_name: get_setting(&state.pool, "blog_name", "Bloogla").await,
        is_page: kind.is_page(),
        admin_path: kind.admin_path(),
        rows,
        filters,
        active_filter: active,
    }
    .into_response()
}

pub async fn new_post(state: State<AppState>) -> Response {
    new_page_for(state, Kind::Post).await
}

pub async fn new_page(state: State<AppState>) -> Response {
    new_page_for(state, Kind::Page).await
}

/// GET /admin/posts/new, /admin/pages/new -> Empty editor.
async fn new_page_for(State(state): State<AppState>, kind: Kind) -> Response {
    let post = Post {
        id: 0,
        title: String::new(),
        slug: String::new(),
        content: String::new(),
        cover_image: None,
        views: 0,
        created_at: String::new(),
        status: "draft".to_string(),
        published_at: String::new(),
        is_page: kind.is_page(),
        reading_time: 0,
        tags: Vec::new(),
    };
    editor(&state, post, &HashSet::new(), None, false).await
}

/// Render the editor for `post` with `selected_tags` checked.
async fn editor(
    state: &AppState,
    post: Post,
    selected_tags: &HashSet<i64>,
    error: Option<String>,
    saved: bool,
) -> Response {
    let tag_checkboxes = crate::tags::get_all_tags(&state.pool)
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
            label: crate::utils::format_display_datetime(&saved_at),
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
            blog_name: get_setting(&state.pool, "blog_name", "Bloogla").await,
            base_url: state.config.base_url.clone(),
            admin_path: kind.admin_path(),
            public_path: (post.id != 0 && is_live(&post))
                .then(|| crate::handlers::public::post_path(&post.slug, post.is_page)),
            published_at_input: to_datetime_local(&post.published_at),
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
pub async fn edit_page(
    State(state): State<AppState>,
    Path(id): Path<i64>,
    Query(query): Query<EditQuery>,
) -> Response {
    let post = or_log(
        sqlx::query_as::<_, Post>(&format!("{POST_SELECT} WHERE id = ?"))
            .bind(id)
            .fetch_optional(&state.pool)
            .await,
        "load post for editing",
    );
    let Some(post) = post else {
        return (StatusCode::NOT_FOUND, Html("Post not found")).into_response();
    };

    let selected: HashSet<i64> = crate::tags::get_tags_for_post(&state.pool, id)
        .await
        .iter()
        .map(|t| t.id)
        .collect();
    editor(&state, post, &selected, None, query.saved.is_some()).await
}

/// Build an unsaved `Post` from submitted values, to re-show the editor after an error.
fn post_from_form(id: i64, is_page: bool, slug: String, form: &PostForm) -> Post {
    Post {
        id,
        title: form.title.clone(),
        slug,
        content: form.content.clone(),
        cover_image: form.cover_image.clone().filter(|c| !c.trim().is_empty()),
        views: 0,
        created_at: String::new(),
        status: normalize_status(form.status.clone()),
        published_at: normalize_published_at(form.published_at.clone()),
        is_page,
        reading_time: 0,
        tags: Vec::new(),
    }
}

pub async fn create_post(state: State<AppState>, form: Form<PostForm>) -> Response {
    create(state, form, Kind::Post).await
}

pub async fn create_page(state: State<AppState>, form: Form<PostForm>) -> Response {
    create(state, form, Kind::Page).await
}

/// POST /admin/posts, /admin/pages -> Create, then continue in the editor.
async fn create(State(state): State<AppState>, Form(form): Form<PostForm>, kind: Kind) -> Response {
    let title = form.title.trim();
    if title.is_empty() {
        let post = post_from_form(
            0,
            kind.is_page(),
            form.slug.clone().unwrap_or_default(),
            &form,
        );
        let tags = form.tag_ids.iter().copied().collect();
        return editor(
            &state,
            post,
            &tags,
            Some("Add a title first.".into()),
            false,
        )
        .await;
    }

    let slug_source = match form.slug.as_deref().map(str::trim) {
        Some(custom) if !custom.is_empty() => custom,
        _ => title,
    };
    let slug = match generate_unique_slug(&state.pool, slug_source, None).await {
        Ok(slug) => slug,
        Err(e) => {
            eprintln!("Failed to generate slug: {e}");
            return database_error();
        }
    };

    let post = post_from_form(0, kind.is_page(), slug, &form);
    let result = sqlx::query(
        "INSERT INTO posts (title, slug, content, cover_image, status, published_at, is_page)
         VALUES (?, ?, ?, ?, ?, ?, ?)",
    )
    .bind(title)
    .bind(&post.slug)
    .bind(&post.content)
    .bind(&post.cover_image)
    .bind(&post.status)
    .bind(&post.published_at)
    .bind(post.is_page)
    .execute(&state.pool)
    .await;

    let id = match result {
        Ok(r) => r.last_insert_rowid(),
        Err(e) => {
            eprintln!("Failed to insert post: {e}");
            return database_error();
        }
    };
    crate::tags::set_post_tags(&state.pool, id, &form.tag_ids).await;

    Redirect::to(&format!("{}/{id}/edit?saved=1", kind.admin_path())).into_response()
}

/// POST /admin/posts/:id/edit -> Save changes and stay in the editor.
pub async fn update(
    State(state): State<AppState>,
    Path(id): Path<i64>,
    Form(form): Form<PostForm>,
) -> Response {
    let existing: Option<(String, bool, String, String)> = or_log(
        sqlx::query_as("SELECT slug, is_page, title, content FROM posts WHERE id = ?")
            .bind(id)
            .fetch_optional(&state.pool)
            .await,
        "load post slug",
    );
    let Some((old_slug, is_page, old_title, old_content)) = existing else {
        return (StatusCode::NOT_FOUND, Html("Post not found")).into_response();
    };
    let kind = if is_page { Kind::Page } else { Kind::Post };

    let title = form.title.trim();
    if title.is_empty() {
        let post = post_from_form(id, is_page, old_slug, &form);
        let tags = form.tag_ids.iter().copied().collect();
        return editor(
            &state,
            post,
            &tags,
            Some("Add a title first.".into()),
            false,
        )
        .await;
    }

    // The URL only changes when the slug field is edited, so links keep working.
    let slug = match form.slug.as_deref().map(str::trim) {
        Some(custom) if !custom.is_empty() && custom != old_slug => {
            match generate_unique_slug(&state.pool, custom, Some(id)).await {
                Ok(slug) => slug,
                Err(e) => {
                    eprintln!("Failed to generate slug: {e}");
                    return database_error();
                }
            }
        }
        _ => old_slug.clone(),
    };

    let post = post_from_form(id, is_page, slug, &form);

    if old_title != title || old_content != post.content {
        save_revision(&state, id, &old_title, &old_content).await;
    }

    let result = sqlx::query(
        "UPDATE posts SET title = ?, slug = ?, content = ?, cover_image = ?, status = ?,
                published_at = ?
         WHERE id = ?",
    )
    .bind(title)
    .bind(&post.slug)
    .bind(&post.content)
    .bind(&post.cover_image)
    .bind(&post.status)
    .bind(&post.published_at)
    .bind(id)
    .execute(&state.pool)
    .await;

    if let Err(e) = result {
        eprintln!("Failed to update post: {e}");
        return database_error();
    }

    if post.slug != old_slug {
        let _ = sqlx::query("DELETE FROM slug_redirects WHERE old_slug = ?")
            .bind(&post.slug)
            .execute(&state.pool)
            .await;
        let _ = sqlx::query(
            "INSERT INTO slug_redirects (old_slug, post_id) VALUES (?, ?)
             ON CONFLICT(old_slug) DO UPDATE SET post_id = excluded.post_id",
        )
        .bind(&old_slug)
        .bind(id)
        .execute(&state.pool)
        .await;
    }

    crate::tags::set_post_tags(&state.pool, id, &form.tag_ids).await;

    Redirect::to(&format!("{}/{id}/edit?saved=1", kind.admin_path())).into_response()
}

/// Revisions kept per post; older ones are deleted.
const MAX_REVISIONS: i64 = 25;

/// Keep the version of a post that is about to be overwritten.
async fn save_revision(state: &AppState, post_id: i64, title: &str, content: &str) {
    let saved =
        sqlx::query("INSERT INTO post_revisions (post_id, title, content) VALUES (?, ?, ?)")
            .bind(post_id)
            .bind(title)
            .bind(content)
            .execute(&state.pool)
            .await;
    if let Err(e) = saved {
        eprintln!("Failed to save revision: {e}");
        return;
    }
    let _ = sqlx::query(
        "DELETE FROM post_revisions WHERE post_id = ? AND id NOT IN
           (SELECT id FROM post_revisions WHERE post_id = ? ORDER BY id DESC LIMIT ?)",
    )
    .bind(post_id)
    .bind(post_id)
    .bind(MAX_REVISIONS)
    .execute(&state.pool)
    .await;
}

/// GET /admin/posts/:id/revisions/:rev -> One earlier version, for restoring in the editor.
pub async fn revision(
    State(state): State<AppState>,
    Path((id, rev)): Path<(i64, i64)>,
) -> Response {
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
pub async fn delete(State(state): State<AppState>, Path(id): Path<i64>) -> StatusCode {
    match sqlx::query("DELETE FROM posts WHERE id = ?")
        .bind(id)
        .execute(&state.pool)
        .await
    {
        Ok(r) if r.rows_affected() > 0 => StatusCode::OK,
        Ok(_) => StatusCode::NOT_FOUND,
        Err(e) => {
            eprintln!("Failed to delete post: {e}");
            StatusCode::INTERNAL_SERVER_ERROR
        }
    }
}

/// POST /admin/preview -> Rendered Markdown for the editor's Preview tab.
pub async fn preview(Form(form): Form<PreviewForm>) -> Html<String> {
    if form.content.trim().is_empty() {
        return Html(r#"<p class="text-muted">Nothing to preview yet.</p>"#.to_string());
    }
    Html(render_safe_markdown(&form.content))
}

fn database_error() -> Response {
    (
        StatusCode::INTERNAL_SERVER_ERROR,
        Html("Something went wrong while saving. Please try again."),
    )
        .into_response()
}
