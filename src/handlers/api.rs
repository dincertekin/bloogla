//! JSON API.
//!
//! Reading published content needs no authentication. Writing uses a personal
//! token (`Authorization: Bearer bl_...`) created on the Profile page; it acts
//! with its owner's role, so an author's token only reaches their own posts.

use crate::app::models::{CurrentUser, Post, PostForm};
use crate::app::state::AppState;
use crate::content::markdown::excerpt;
use crate::content::seo::absolute_url;
use crate::content::text::to_iso8601;
use crate::db::fields::{self, Fields};
use crate::db::posts::{self, SaveError, LISTED_POST_FILTER, POST_SELECT};
use crate::db::{or_log, tags};
use crate::server::routes::post_path;

use axum::extract::{Path, Query, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::{Extension, Json};
use serde::{Deserialize, Serialize};
use serde_json::json;

const MAX_PER_PAGE: i64 = 50;

/// A JSON error: `{"error": "..."}`.
pub fn error(status: StatusCode, message: &str) -> Response {
    (status, Json(json!({ "error": message }))).into_response()
}

fn save_error(e: SaveError) -> Response {
    match e {
        SaveError::NotFound => error(StatusCode::NOT_FOUND, "Post not found"),
        SaveError::Forbidden => error(StatusCode::FORBIDDEN, "Your role doesn't allow this"),
        SaveError::MissingTitle => error(StatusCode::UNPROCESSABLE_ENTITY, "A title is required"),
        SaveError::Database => error(StatusCode::INTERNAL_SERVER_ERROR, "Could not save"),
    }
}

#[derive(Serialize)]
struct ApiTag {
    name: String,
    slug: String,
}

#[derive(Serialize)]
struct ApiPost {
    id: i64,
    kind: &'static str,
    title: String,
    slug: String,
    url: String,
    status: String,
    published_at: String,
    excerpt: String,
    cover_image: Option<String>,
    author: Option<String>,
    tags: Vec<ApiTag>,
    fields: Fields,
    /// Markdown source; only on single-post responses.
    #[serde(skip_serializing_if = "Option::is_none")]
    content: Option<String>,
    /// Rendered, sanitized HTML; only on single-post responses.
    #[serde(skip_serializing_if = "Option::is_none")]
    html: Option<String>,
}

/// `html` is the rendered content, given only for single-post responses.
fn to_api(state: &AppState, post: Post, html: Option<String>) -> ApiPost {
    let full = html.is_some();
    let base = &state.config.base_url;
    ApiPost {
        id: post.id,
        kind: if post.is_page { "page" } else { "post" },
        url: format!("{base}{}", post_path(&post.slug, post.is_page)),
        status: post.status,
        published_at: to_iso8601(&post.published_at),
        excerpt: excerpt(&post.content, 200),
        cover_image: post.cover_image.map(|c| absolute_url(&c, base)),
        author: post.author_name,
        tags: post
            .tags
            .into_iter()
            .map(|t| ApiTag {
                name: t.name,
                slug: t.slug,
            })
            .collect(),
        fields: post.fields,
        html,
        content: full.then_some(post.content),
        title: post.title,
        slug: post.slug,
    }
}

#[derive(Deserialize)]
pub struct ListQuery {
    page: Option<i64>,
    per_page: Option<i64>,
    tag: Option<String>,
}

/// GET /api/posts -> Published posts, newest first. `?tag=slug&page=2&per_page=20`
pub async fn list_posts(State(state): State<AppState>, Query(query): Query<ListQuery>) -> Response {
    let per_page = query.per_page.unwrap_or(10).clamp(1, MAX_PER_PAGE);
    let page = query.page.unwrap_or(1).max(1);
    let tag = query.tag.unwrap_or_default();
    let filter = format!(
        "{LISTED_POST_FILTER} AND (? = '' OR id IN (
            SELECT pt.post_id FROM post_tags pt JOIN tags t ON t.id = pt.tag_id WHERE t.slug = ?))"
    );

    let total: i64 = or_log(
        sqlx::query_scalar(&format!("SELECT COUNT(*) FROM posts WHERE {filter}"))
            .bind(&tag)
            .bind(&tag)
            .fetch_one(&state.pool)
            .await,
        "api count posts",
    );
    let mut posts = or_log(
        sqlx::query_as::<_, Post>(&format!(
            "{POST_SELECT} WHERE {filter} ORDER BY published_at DESC LIMIT ? OFFSET ?"
        ))
        .bind(&tag)
        .bind(&tag)
        .bind(per_page)
        .bind((page - 1) * per_page)
        .fetch_all(&state.pool)
        .await,
        "api list posts",
    );

    posts::load_tags_and_fields(&state.pool, &mut posts).await;

    Json(json!({
        "posts": posts.into_iter().map(|p| to_api(&state, p, None)).collect::<Vec<_>>(),
        "page": page,
        "per_page": per_page,
        "total": total,
        "total_pages": ((total + per_page - 1) / per_page).max(1),
    }))
    .into_response()
}

async fn public_by_slug(state: &AppState, slug: &str, is_page: bool) -> Response {
    match posts::find_public(&state.pool, slug, is_page).await {
        Some(post) => full_post(state, post, StatusCode::OK).await,
        None => error(StatusCode::NOT_FOUND, "Not found"),
    }
}

/// One post with its tags, fields and content as the response body.
async fn full_post(state: &AppState, mut post: Post, status: StatusCode) -> Response {
    posts::load_tags_and_fields(&state.pool, std::slice::from_mut(&mut post)).await;
    let html = crate::content::render(state, &post.content).await;
    (status, Json(to_api(state, post, Some(html)))).into_response()
}

/// GET /api/posts/:slug -> One published post with its content.
pub async fn get_post(State(state): State<AppState>, Path(slug): Path<String>) -> Response {
    public_by_slug(&state, &slug, false).await
}

/// GET /api/pages/:slug -> One published page with its content.
pub async fn get_page(State(state): State<AppState>, Path(slug): Path<String>) -> Response {
    public_by_slug(&state, &slug, true).await
}

/// GET /api/tags -> Tags with their number of published posts.
pub async fn list_tags(State(state): State<AppState>) -> Response {
    let rows: Vec<(String, String, i64)> = or_log(
        sqlx::query_as(&format!(
            "SELECT t.name, t.slug,
                    (SELECT COUNT(*) FROM post_tags pt JOIN posts ON posts.id = pt.post_id
                     WHERE pt.tag_id = t.id AND {LISTED_POST_FILTER})
             FROM tags t ORDER BY t.name"
        ))
        .fetch_all(&state.pool)
        .await,
        "api list tags",
    );
    let base = &state.config.base_url;
    Json(json!({
        "tags": rows.into_iter().map(|(name, slug, count)| json!({
            "name": name,
            "url": format!("{base}/tag/{slug}"),
            "slug": slug,
            "post_count": count,
        })).collect::<Vec<_>>()
    }))
    .into_response()
}

/// Any other /api address: a JSON 404 instead of the theme's HTML page.
pub async fn not_found() -> Response {
    error(StatusCode::NOT_FOUND, "No such API endpoint")
}

/// GET /api/me -> Who the token belongs to.
pub async fn me(Extension(me): Extension<CurrentUser>) -> Response {
    Json(json!({
        "id": me.id,
        "name": me.name,
        "email": me.email,
        "role": me.role.as_str(),
    }))
    .into_response()
}

/// Body for creating or updating a post. On update, missing fields keep their values.
#[derive(Deserialize)]
pub struct PostInput {
    title: Option<String>,
    content: Option<String>,
    slug: Option<String>,
    /// `draft`, `published` or `scheduled`.
    status: Option<String>,
    /// UTC, e.g. `2026-10-02T09:00:00`.
    published_at: Option<String>,
    cover_image: Option<String>,
    /// Tag names or slugs.
    tags: Option<Vec<String>>,
    /// Custom fields; on update, sending this replaces all of them.
    fields: Option<Fields>,
    /// Create a standalone page instead of a post.
    #[serde(default)]
    page: bool,
}

/// Resolve tag names to ids; editors and admins create missing tags.
async fn resolve_tags(
    state: &AppState,
    me: &CurrentUser,
    names: &[String],
) -> Result<Vec<i64>, Response> {
    let mut ids = Vec::new();
    for name in names.iter().map(|n| n.trim()).filter(|n| !n.is_empty()) {
        match tags::find_id(&state.pool, name).await {
            Some(id) => ids.push(id),
            None if me.can_edit_all() => match tags::create(&state.pool, name).await {
                Ok(id) => ids.push(id),
                Err(_) => {
                    return Err(error(
                        StatusCode::INTERNAL_SERVER_ERROR,
                        "Could not create tag",
                    ))
                }
            },
            None => {
                return Err(error(
                    StatusCode::UNPROCESSABLE_ENTITY,
                    &format!("There's no tag '{name}'. Only editors and admins can create tags."),
                ))
            }
        }
    }
    Ok(ids)
}

/// POST /api/posts -> Create a post (or a page with `"page": true`).
pub async fn create_post(
    State(state): State<AppState>,
    Extension(me): Extension<CurrentUser>,
    Json(input): Json<PostInput>,
) -> Response {
    let tag_ids = match resolve_tags(&state, &me, input.tags.as_deref().unwrap_or_default()).await {
        Ok(ids) => ids,
        Err(response) => return response,
    };
    let fields = fields::clean(input.fields.unwrap_or_default());
    let form = PostForm {
        title: input.title.unwrap_or_default(),
        slug: input.slug,
        content: input.content.unwrap_or_default(),
        cover_image: input.cover_image,
        status: input.status,
        published_at: input.published_at,
        tag_ids,
        field_keys: fields.keys().cloned().collect(),
        field_values: fields.values().cloned().collect(),
        send_newsletter: None,
    };
    match posts::create(&state.pool, &me, input.page, &form).await {
        Ok(id) => {
            crate::services::webmention::send_for_post(&state, id);
            match posts::find(&state.pool, id).await {
                Some(post) => full_post(&state, post, StatusCode::CREATED).await,
                None => error(
                    StatusCode::INTERNAL_SERVER_ERROR,
                    "Created, but could not load it",
                ),
            }
        }
        Err(e) => save_error(e),
    }
}

/// PUT /api/posts/:id -> Update a post; fields left out keep their current values.
pub async fn update(
    State(state): State<AppState>,
    Extension(me): Extension<CurrentUser>,
    Path(id): Path<i64>,
    Json(input): Json<PostInput>,
) -> Response {
    let Some(mut current) = posts::find(&state.pool, id).await else {
        return save_error(SaveError::NotFound);
    };
    posts::load_tags_and_fields(&state.pool, std::slice::from_mut(&mut current)).await;
    let tag_ids = match &input.tags {
        Some(names) => match resolve_tags(&state, &me, names).await {
            Ok(ids) => ids,
            Err(response) => return response,
        },
        None => current.tags.iter().map(|t| t.id).collect(),
    };
    let fields = match input.fields {
        Some(fields) => fields::clean(fields),
        None => current.fields.clone(),
    };
    let form = PostForm {
        title: input.title.unwrap_or(current.title),
        slug: Some(input.slug.unwrap_or(current.slug)),
        content: input.content.unwrap_or(current.content),
        cover_image: input.cover_image.or(current.cover_image),
        status: Some(input.status.unwrap_or(current.status)),
        published_at: Some(input.published_at.unwrap_or(current.published_at)),
        tag_ids,
        field_keys: fields.keys().cloned().collect(),
        field_values: fields.values().cloned().collect(),
        send_newsletter: None,
    };
    match posts::update(&state.pool, &me, id, &form).await {
        Ok(()) => {
            crate::services::webmention::send_for_post(&state, id);
            match posts::find(&state.pool, id).await {
                Some(post) => full_post(&state, post, StatusCode::OK).await,
                None => error(
                    StatusCode::INTERNAL_SERVER_ERROR,
                    "Saved, but could not load it",
                ),
            }
        }
        Err(e) => save_error(e),
    }
}

/// DELETE /api/posts/:id -> Delete a post.
pub async fn delete(
    State(state): State<AppState>,
    Extension(me): Extension<CurrentUser>,
    Path(id): Path<i64>,
) -> Response {
    match posts::delete(&state.pool, &me, id).await {
        Ok(()) => StatusCode::NO_CONTENT.into_response(),
        Err(e) => save_error(e),
    }
}
