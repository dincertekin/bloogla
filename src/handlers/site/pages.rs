//! Theme pages: home, posts, standalone pages, tags and search, plus
//! redirects from old addresses.

use super::{analytics, base_context, noindex_head, not_found, seo_head};
use crate::app::models::{Pagination, Post};
use crate::app::state::AppState;
use crate::content::markdown::excerpt;
use crate::content::seo::{SeoKind, SeoMeta};
use crate::content::text::{
    display_date, escape_html, percent_decode, reading_time, slugify, url_encode,
};
use crate::db::posts::{self, LISTED_POST_FILTER, POST_SELECT, PUBLIC_POST_FILTER};
use crate::db::settings::{self, CommentMode, Settings};
use crate::db::{or_log, tags};
use crate::server::routes::post_path;
use crate::services::themes;

use axum::extract::{Path, Query, State};
use axum::http::{HeaderMap, Method, StatusCode, Uri};
use axum::response::{IntoResponse, Redirect, Response};
use serde::Deserialize;
use tower_sessions::Session;

/// Characters of plain text in listing excerpts and meta descriptions.
const EXCERPT_CHARS: usize = 180;
const META_DESCRIPTION_CHARS: usize = 160;

#[derive(Deserialize)]
pub struct ListingQuery {
    page: Option<i64>,
    /// Result of the newsletter form.
    subscribe: Option<String>,
}

#[derive(Deserialize)]
pub struct PostQuery {
    /// Result of sending a comment (`pending`, `slow`, `invalid`).
    comment: Option<String>,
    /// Result of the newsletter form.
    subscribe: Option<String>,
}

#[derive(Deserialize)]
pub struct SearchQuery {
    q: Option<String>,
    page: Option<i64>,
}

/// GET / -> Newest posts.
pub async fn home(State(state): State<AppState>, Query(query): Query<ListingQuery>) -> Response {
    let site = settings::load(&state.pool).await;
    let Some((posts, pagination)) = fetch_listing(
        &state,
        &site,
        LISTED_POST_FILTER,
        "",
        &[],
        query.page,
        |p| with_page("/", p),
    )
    .await
    else {
        return not_found(&state).await;
    };

    let mut context = base_context(&state, &site).await;
    context.insert("posts", &posts);
    context.insert("pagination", &pagination);
    context.insert(
        "subscribe_notice",
        &super::newsletter::notice(query.subscribe.as_deref()).map(|n| site.language.t(n)),
    );
    context.insert("return_to", "/");

    themes::render(&state, &["templates/index.html"], context, StatusCode::OK).await
}

/// GET /post/:slug -> One post.
pub async fn show_post(
    State(state): State<AppState>,
    Path(slug): Path<String>,
    Query(query): Query<PostQuery>,
    headers: HeaderMap,
    session: Session,
) -> Response {
    if let Some(post) = posts::find_public(&state.pool, &slug, false).await {
        let notices = (query.comment.as_deref(), query.subscribe.as_deref());
        return render_post(&state, post, &headers, &session, notices).await;
    }
    match redirect_to_slug(&state, &slug).await {
        Some(redirect) => redirect,
        None => not_found(&state).await,
    }
}

/// GET /:slug -> A standalone page (About, Contact...).
///
/// A post with this slug is redirected to its post URL, which keeps
/// WordPress-style `/post-name` links working after an import.
pub async fn show_page(
    State(state): State<AppState>,
    Path(slug): Path<String>,
    headers: HeaderMap,
    session: Session,
) -> Response {
    if let Some(page) = posts::find_public(&state.pool, &slug, true).await {
        return render_post(&state, page, &headers, &session, (None, None)).await;
    }
    match redirect_to_slug(&state, &slug).await {
        Some(redirect) => redirect,
        None => not_found(&state).await,
    }
}

/// GET /tag/:slug -> Posts with a tag.
pub async fn tag(
    State(state): State<AppState>,
    Path(slug): Path<String>,
    Query(query): Query<ListingQuery>,
) -> Response {
    let Some(tag) = tags::find_by_slug(&state.pool, &slug).await else {
        return not_found(&state).await;
    };

    let site = settings::load(&state.pool).await;
    let base_path = format!("/tag/{}", url_encode(&tag.slug));
    let Some((posts, pagination)) = fetch_listing(
        &state,
        &site,
        LISTED_POST_FILTER,
        "AND id IN (SELECT pt.post_id FROM post_tags pt
                    JOIN tags t ON t.id = pt.tag_id WHERE t.slug = ?)",
        &[&tag.slug],
        query.page,
        |p| with_page(&base_path, p),
    )
    .await
    else {
        return not_found(&state).await;
    };

    let mut context = base_context(&state, &site).await;
    context.insert("tag_name", &tag.name);
    context.insert("posts", &posts);
    context.insert("pagination", &pagination);

    themes::render(&state, &["templates/tag.html"], context, StatusCode::OK).await
}

/// GET /search?q=... -> Full-text search, shown with the home page template.
pub async fn search(State(state): State<AppState>, Query(query): Query<SearchQuery>) -> Response {
    let search_term = query.q.unwrap_or_default().trim().to_string();
    if search_term.is_empty() {
        return Redirect::to("/").into_response();
    }

    let site = settings::load(&state.pool).await;
    let base_path = format!("/search?q={}", url_encode(&search_term));
    // A query of only punctuation matches nothing.
    let fts_query = posts::search_query(&search_term).unwrap_or_else(|| "\"\"".to_string());
    // Search finds pages too (docs, About...); cards link with `post.url`.
    let Some((posts, pagination)) = fetch_listing(
        &state,
        &site,
        PUBLIC_POST_FILTER,
        "AND id IN (SELECT rowid FROM posts_fts WHERE posts_fts MATCH ?)",
        &[&fts_query],
        query.page,
        |p| with_page(&base_path, p),
    )
    .await
    else {
        return not_found(&state).await;
    };

    let mut context = base_context(&state, &site).await;
    context.insert("seo_head", &noindex_head(&state, &site));
    context.insert("posts", &posts);
    context.insert("pagination", &pagination);
    context.insert("search_query", &search_term);

    themes::render(&state, &["templates/index.html"], context, StatusCode::OK).await
}

/// Any other address. Old WordPress permalinks (`/post-name/`,
/// `/2020/05/post-name/`) end with the slug, so the last part of the path is
/// tried before giving up.
pub async fn fallback(State(state): State<AppState>, method: Method, uri: Uri) -> Response {
    if uri.path().starts_with("/api/") {
        return crate::handlers::api::not_found().await;
    }
    if method == Method::GET {
        let last_segment = uri.path().trim_end_matches('/').rsplit('/').next();
        if let Some(segment) = last_segment.filter(|s| !s.is_empty()) {
            if let Some(redirect) = redirect_to_slug(&state, &percent_decode(segment)).await {
                return redirect;
            }
        }
    }
    not_found(&state).await
}

/// Render a post or page with its SEO tags, comments and newsletter form.
/// `notices` are the results of the comment and newsletter forms.
async fn render_post(
    state: &AppState,
    mut post: Post,
    headers: &HeaderMap,
    session: &Session,
    (comment_notice, subscribe_notice): (Option<&str>, Option<&str>),
) -> Response {
    if analytics::is_countable_view(headers, session).await {
        let referrer = analytics::referrer_host(headers, &state.config.base_url);
        analytics::record_view(state, post.id, referrer);
    }

    let site = settings::load(&state.pool).await;
    let lang = site.language;
    let content_html = crate::content::render(state, &post.content).await;
    let description = excerpt(&post.content, META_DESCRIPTION_CHARS);
    post.reading_time = reading_time(&post.content);
    posts::load_tags_and_fields(&state.pool, std::slice::from_mut(&mut post)).await;

    let path = post_path(&post.slug, post.is_page);
    post.url = path.clone();
    let canonical_url = format!("{}{path}", state.config.base_url);
    let tag_names: Vec<&str> = post.tags.iter().map(|t| t.name.as_str()).collect();
    let seo_head = seo_head(
        state,
        &site,
        &SeoMeta {
            title: &post.title,
            description: &description,
            canonical_url: &canonical_url,
            image: post.cover_image.as_deref(),
            kind: if post.is_page {
                SeoKind::Website
            } else {
                SeoKind::Post {
                    published_at: &post.published_at,
                    tags: tag_names,
                    author: post.author_name.as_deref(),
                }
            },
            noindex: false,
        },
    );
    // Themes show `created_at` as the post's date.
    post.created_at = display_date(lang, &post.published_at);

    let mut context = base_context(state, &site).await;
    context.insert("seo_head", &seo_head);
    context.insert("content_html", &content_html);
    context.insert(
        "subscribe_notice",
        &super::newsletter::notice(subscribe_notice).map(|n| lang.t(n)),
    );
    context.insert("return_to", &path);

    // Comments are for posts only. Themes check `comments_enabled`.
    let comments_enabled = !post.is_page && site.comments != CommentMode::Off;
    context.insert("comments_enabled", &comments_enabled);
    if comments_enabled {
        context.insert(
            "comments",
            &super::comments::approved_for(state, post.id, lang).await,
        );
        context.insert("comment_action", &format!("{path}/comments"));
        context.insert(
            "comment_notice",
            &super::comments::notice(comment_notice).map(|n| lang.t(n)),
        );
    }

    if !post.is_page {
        context.insert("more_posts", &more_posts(state, &site, post.id).await);
    }

    let templates: &[&str] = if post.is_page {
        &["templates/page.html", "templates/post.html"]
    } else {
        &["templates/post.html"]
    };
    context.insert("post", &post);
    themes::render(state, templates, context, StatusCode::OK).await
}

/// Permanent redirect to the public post or page currently or formerly at `slug`.
///
/// Imported slugs are transliterated (`dünya` → `dunya`), so that form is tried too.
async fn redirect_to_slug(state: &AppState, slug: &str) -> Option<Response> {
    let slugified = slugify(slug);
    let candidates = if slugified == slug {
        vec![slug]
    } else {
        vec![slug, slugified.as_str()]
    };

    for candidate in candidates {
        for is_page in [false, true] {
            if let Some(post) = posts::find_public(&state.pool, candidate, is_page).await {
                return Some(
                    Redirect::permanent(&post_path(&post.slug, post.is_page)).into_response(),
                );
            }
        }
        if let Some(redirect) = redirect_old_slug(state, candidate).await {
            return Some(redirect);
        }
    }
    None
}

/// If `slug` is an old URL of a public post, redirect to its current URL.
async fn redirect_old_slug(state: &AppState, slug: &str) -> Option<Response> {
    let target: Option<(String, bool)> = or_log(
        sqlx::query_as(&format!(
            "SELECT slug, is_page FROM posts
             WHERE id = (SELECT post_id FROM slug_redirects WHERE old_slug = ?)
               AND {PUBLIC_POST_FILTER}"
        ))
        .bind(slug)
        .fetch_optional(&state.pool)
        .await,
        "slug redirect lookup",
    );

    target.map(|(new_slug, is_page)| {
        Redirect::permanent(&post_path(&new_slug, is_page)).into_response()
    })
}

/// One page of posts matching `base_filter` (which posts are public, e.g.
/// [`LISTED_POST_FILTER`]) and `extra_filter` (with `binds` for its `?`s).
///
/// `page_url` builds the link for a page number. Returns `None` when `page` is
/// past the last page.
async fn fetch_listing(
    state: &AppState,
    site: &Settings,
    base_filter: &str,
    extra_filter: &str,
    binds: &[&str],
    page: Option<i64>,
    page_url: impl Fn(i64) -> String,
) -> Option<(Vec<Post>, Pagination)> {
    let per_page = site.posts_per_page;
    let current = page.unwrap_or(1).max(1);
    let filter = format!("{base_filter} {extra_filter}");

    let count_sql = format!("SELECT COUNT(*) FROM posts WHERE {filter}");
    let mut count_query = sqlx::query_scalar::<_, i64>(&count_sql);
    for bind in binds {
        count_query = count_query.bind(*bind);
    }
    let total = or_log(count_query.fetch_one(&state.pool).await, "count listing");

    let total_pages = ((total + per_page - 1) / per_page).max(1);
    if current > total_pages {
        return None;
    }

    let list_sql =
        format!("{POST_SELECT} WHERE {filter} ORDER BY published_at DESC LIMIT ? OFFSET ?");
    let mut list_query = sqlx::query_as::<_, Post>(&list_sql);
    for bind in binds {
        list_query = list_query.bind(*bind);
    }
    let mut posts = or_log(
        list_query
            .bind(per_page)
            .bind((current - 1) * per_page)
            .fetch_all(&state.pool)
            .await,
        "fetch listing",
    );

    as_cards(state, site, &mut posts).await;

    let pagination = Pagination {
        current,
        total_pages,
        prev_url: (current > 1).then(|| page_url(current - 1)),
        next_url: (current < total_pages).then(|| page_url(current + 1)),
    };
    Some((posts, pagination))
}

/// Ready posts for a list: post cards show a date, reading time, a short
/// excerpt, tags and fields.
async fn as_cards(state: &AppState, site: &Settings, posts: &mut [Post]) {
    for post in posts.iter_mut() {
        post.url = post_path(&post.slug, post.is_page);
        post.reading_time = reading_time(&post.content);
        post.created_at = display_date(site.language, &post.published_at);
        // Themes print the excerpt with `| safe`, so it must be escaped here.
        post.content = escape_html(&excerpt(&post.content, EXCERPT_CHARS));
    }
    posts::load_tags_and_fields(&state.pool, posts).await;
}

/// The newest listed posts other than `exclude_id`, as cards ("More videos",
/// "Read next").
async fn more_posts(state: &AppState, site: &Settings, exclude_id: i64) -> Vec<Post> {
    let sql = format!(
        "{POST_SELECT} WHERE {LISTED_POST_FILTER} AND id != ? ORDER BY published_at DESC LIMIT 6"
    );
    let mut posts = or_log(
        sqlx::query_as::<_, Post>(&sql)
            .bind(exclude_id)
            .fetch_all(&state.pool)
            .await,
        "fetch more posts",
    );
    as_cards(state, site, &mut posts).await;
    posts
}

/// Add `page=N` to a URL, leaving it out for the first page.
fn with_page(path: &str, page: i64) -> String {
    let separator = if path.contains('?') { '&' } else { '?' };
    if page <= 1 {
        path.to_string()
    } else {
        format!("{path}{separator}page={page}")
    }
}
