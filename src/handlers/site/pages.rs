//! Theme pages: home, posts, standalone pages, tags and search, plus
//! redirects from old addresses.

use super::{analytics, base_context, noindex_head, not_found, seo_head};
use crate::app::models::{Pagination, Post};
use crate::app::state::AppState;
use crate::content::markdown::excerpt;
use crate::content::seo::{SeoKind, SeoMeta};
use crate::content::text::{display_date, escape_html, reading_time, url_decode, url_encode};
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
}

#[derive(Deserialize)]
pub struct PostQuery {
    /// Result of sending a comment (`pending`, `slow`, `invalid`).
    comment: Option<String>,
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
    // Page 1 is the site itself; later pages have their own address.
    if query.page.unwrap_or(1) > 1 {
        let address = with_page("/", query.page.unwrap_or(1));
        let head = listing_head(&state, &site, &site.blog_name, &address);
        context.insert("seo_head", &head);
    }

    themes::render(&state, &["templates/index.html"], context, StatusCode::OK).await
}

/// The SEO tags for a list of posts at `address` (a tag, or a later page of
/// the home page). Its canonical link is its own address, page included;
/// pointing at the home page would tell search engines it's a copy of the
/// home page, and they'd leave it out.
fn listing_head(state: &AppState, site: &Settings, title: &str, address: &str) -> String {
    let canonical_url = format!("{}{address}", state.config.base_url);
    seo_head(
        state,
        site,
        &SeoMeta {
            title,
            description: &site.blog_description,
            canonical_url: &canonical_url,
            image: None,
            kind: SeoKind::Website,
            noindex: false,
        },
    )
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
        return render_post(&state, post, &headers, &session, query.comment.as_deref()).await;
    }
    match redirect_to_slug(&state, &slug).await {
        Some(redirect) => redirect,
        None => not_found(&state).await,
    }
}

/// GET /:slug -> A standalone page (About, Contact...).
///
/// A post with this slug is redirected to its post URL, so a link that
/// leaves out `/post/` still works.
pub async fn show_page(
    State(state): State<AppState>,
    Path(slug): Path<String>,
    headers: HeaderMap,
    session: Session,
) -> Response {
    if let Some(page) = posts::find_public(&state.pool, &slug, true).await {
        return render_post(&state, page, &headers, &session, None).await;
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
    let Some(tag) = tags::find_listed_by_slug(&state.pool, &slug).await else {
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
    let address = with_page(&base_path, query.page.unwrap_or(1));
    context.insert(
        "seo_head",
        &listing_head(&state, &site, &tag.name, &address),
    );
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

/// Any other address: the theme's "not found" page.
///
/// Old WordPress addresses end with the post's name (`/2024/05/lemon-cake/`,
/// `/lemon-cake/`) or a category's (`/category/desserts/`). After an import
/// they lead to the post or tag here instead of "not found", which keeps
/// visitors and search rankings.
pub async fn fallback(State(state): State<AppState>, method: Method, uri: Uri) -> Response {
    if method == Method::GET {
        let mut parts = uri.path().trim_end_matches('/').rsplit('/');
        if let Some(last) = parts.next().filter(|part| !part.is_empty()) {
            let last = url_decode(last);
            if matches!(parts.next(), Some("category" | "tag")) {
                if let Some(tag) = tags::find_listed_by_slug(&state.pool, &last).await {
                    let address = format!("/tag/{}", url_encode(&tag.slug));
                    return Redirect::permanent(&address).into_response();
                }
            }
            if let Some(redirect) = redirect_to_slug(&state, &last).await {
                return redirect;
            }
        }
    }
    not_found(&state).await
}

/// Render a post or page with its SEO tags and comments.
/// `comment_notice` is the result of sending the comment form.
async fn render_post(
    state: &AppState,
    mut post: Post,
    headers: &HeaderMap,
    session: &Session,
    comment_notice: Option<&str>,
) -> Response {
    if analytics::is_countable_view(headers, session).await {
        let referrer = analytics::referrer_host(headers, &state.config.base_url);
        analytics::record_view(post.id, referrer);
    }

    let site = settings::load(&state.pool).await;
    let lang = site.language;
    let content_html = crate::content::render(state, &post.content).await;
    let content_html = crate::content::with_image_copies(&state.pool, &content_html).await;
    let description = excerpt(&post.content, META_DESCRIPTION_CHARS);
    post.reading_time = reading_time(&post.content);
    posts::load_tags(&state.pool, std::slice::from_mut(&mut post)).await;

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
    post.created_at = display_date(lang, site.timezone, &post.published_at);

    let mut context = base_context(state, &site).await;
    context.insert("seo_head", &seo_head);
    context.insert("content_html", &content_html);

    // The older and newer post, for links at the end (empty on pages).
    let (previous_post, next_post) = if post.is_page {
        (None, None)
    } else {
        posts::neighbours(&state.pool, &post).await
    };
    context.insert("previous_post", &previous_post);
    context.insert("next_post", &next_post);

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

    let templates: &[&str] = if post.is_page {
        &["templates/page.html", "templates/post.html"]
    } else {
        &["templates/post.html"]
    };
    context.insert("post", &post);
    themes::render(state, templates, context, StatusCode::OK).await
}

/// Permanent redirect to the public post or page currently or formerly at `slug`.
async fn redirect_to_slug(state: &AppState, slug: &str) -> Option<Response> {
    for is_page in [false, true] {
        if let Some(post) = posts::find_public(&state.pool, slug, is_page).await {
            return Some(Redirect::permanent(&post_path(&post.slug, post.is_page)).into_response());
        }
    }
    redirect_old_slug(state, slug).await
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
        post.created_at = display_date(site.language, site.timezone, &post.published_at);
        // Themes print the excerpt with `| safe`, so it must be escaped here.
        post.content = escape_html(&excerpt(&post.content, EXCERPT_CHARS));
    }
    posts::load_tags(&state.pool, posts).await;
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
