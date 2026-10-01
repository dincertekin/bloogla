use crate::models::{
    PageQuery, Pagination, Post, SearchQuery, Tag, LISTED_POST_FILTER, POST_SELECT,
    PUBLIC_POST_FILTER,
};
use crate::seo::{render_head, SeoKind, SeoMeta, SiteInfo};
use crate::utils::{
    excerpt, get_setting, or_log, parse_menu, percent_decode, to_rfc2822, url_encode, xml_escape,
};
use crate::AppState;
use tower_sessions::Session;

use axum::{
    extract::{Path, Query, State},
    http::{header, HeaderMap, Method, StatusCode, Uri},
    response::{Html, IntoResponse, Redirect, Response},
};

/// Characters of plain text shown in listing excerpts and meta descriptions.
const EXCERPT_CHARS: usize = 180;
const META_DESCRIPTION_CHARS: usize = 160;

/// Render the first template of the active theme that exists among `candidates`
/// (paths relative to the theme folder, e.g. `templates/page.html`).
///
/// Templates are hot-reloaded on every request in debug builds.
pub async fn render_theme(
    state: &AppState,
    candidates: &[&str],
    context: &tera::Context,
    status: StatusCode,
) -> Response {
    let mut active_theme = get_setting(&state.pool, "active_theme", "default").await;

    if cfg!(debug_assertions) {
        if let Ok(mut tera) = state.tera.write() {
            let _ = tera.full_reload();
        }
    }

    let tera = match state.tera.read() {
        Ok(tera) => tera,
        Err(_) => return theme_error("template engine unavailable"),
    };
    // A removed or broken theme falls back to the bundled one.
    if !tera
        .get_template_names()
        .any(|t| t.starts_with(&format!("{active_theme}/")))
    {
        active_theme = "default".to_string();
    }
    let template = candidates
        .iter()
        .map(|path| format!("{active_theme}/{path}"))
        .find(|name| tera.get_template_names().any(|t| t == name));

    let Some(template) = template else {
        return if status == StatusCode::NOT_FOUND {
            (status, Html("<h1>404 Not Found</h1>")).into_response()
        } else {
            theme_error(&format!("theme '{active_theme}' is missing {candidates:?}"))
        };
    };

    match tera.render(&template, context) {
        Ok(html) => (status, Html(html)).into_response(),
        Err(err) => theme_error(&format!("{template}: {err:?}")),
    }
}

fn theme_error(detail: &str) -> Response {
    eprintln!("Theme Rendering Error: {detail}");
    (
        StatusCode::INTERNAL_SERVER_ERROR,
        Html("<h1>500 - Theme Error</h1><p>Failed to render theme template.</p>"),
    )
        .into_response()
}

/// Context every theme template receives: site info, menu, tags and default SEO tags.
async fn base_context(state: &AppState) -> tera::Context {
    let pool = &state.pool;
    let blog_name = get_setting(pool, "blog_name", "Bloogla").await;
    let blog_description = get_setting(pool, "blog_description", "").await;
    let menu = parse_menu(&get_setting(pool, "nav_menu", "").await);
    let tags = crate::tags::get_all_tags(pool).await;
    let show_views = get_setting(pool, "show_views", "false").await == "true";

    let seo_head = seo_head(
        state,
        &SeoMeta {
            title: &blog_name,
            description: &blog_description,
            canonical_url: &state.config.base_url,
            image: None,
            kind: SeoKind::Website,
            noindex: false,
        },
    )
    .await;

    let mut context = tera::Context::new();
    context.insert("blog_name", &blog_name);
    context.insert("blog_description", &blog_description);
    context.insert("base_url", &state.config.base_url);
    context.insert("menu", &menu);
    context.insert("tags", &tags);
    context.insert("search_query", "");
    context.insert("show_views", &show_views);
    context.insert("seo_head", &seo_head);
    context
}

async fn seo_head(state: &AppState, meta: &SeoMeta<'_>) -> String {
    let pool = &state.pool;
    let blog_name = get_setting(pool, "blog_name", "Bloogla").await;
    let publisher_type = get_setting(pool, "publisher_type", "Person").await;
    let site_icon = get_setting(pool, "site_icon", "").await;
    let publisher_name = get_setting(pool, "publisher_name", "").await;
    // Older installs stored the admin email here; never publish it.
    let publisher_name = if publisher_name.trim().is_empty() || publisher_name.contains('@') {
        blog_name.clone()
    } else {
        publisher_name
    };

    render_head(
        meta,
        &SiteInfo {
            name: &blog_name,
            base_url: &state.config.base_url,
            publisher_type: &publisher_type,
            publisher_name: &publisher_name,
            icon: (!site_icon.is_empty()).then_some(site_icon.as_str()),
        },
    )
}

/// Fill in reading time, display date, excerpt and tags for post cards.
async fn format_posts_for_listing(pool: &sqlx::Pool<sqlx::Sqlite>, posts: Vec<Post>) -> Vec<Post> {
    let mut formatted: Vec<Post> = posts
        .into_iter()
        .map(|mut post| {
            post.reading_time = crate::utils::calculate_reading_time(&post.content);
            post.created_at = crate::utils::format_display_date(&post.published_at);
            // Themes print the excerpt with `| safe`, so it must be HTML-escaped here.
            post.content = xml_escape(&excerpt(&post.content, EXCERPT_CHARS));
            post
        })
        .collect();

    let post_ids: Vec<i64> = formatted.iter().map(|p| p.id).collect();
    let tag_map = crate::tags::get_tags_for_posts(pool, &post_ids).await;
    for post in formatted.iter_mut() {
        post.tags = tag_map.get(&post.id).cloned().unwrap_or_default();
    }

    formatted
}

/// Fetch one page of listed posts matching `extra_filter` (bound to `binds`).
///
/// `page_url` builds the link for a page number. Returns `None` when `page` is
/// past the last page.
async fn fetch_listing(
    state: &AppState,
    extra_filter: &str,
    binds: &[&str],
    page: Option<i64>,
    page_url: impl Fn(i64) -> String,
) -> Option<(Vec<Post>, Pagination)> {
    let per_page = get_setting(&state.pool, "posts_per_page", "10")
        .await
        .parse::<i64>()
        .unwrap_or(10)
        .clamp(1, 100);
    let current = page.unwrap_or(1).max(1);

    let filter = format!("{LISTED_POST_FILTER} {extra_filter}");

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
    let posts = or_log(
        list_query
            .bind(per_page)
            .bind((current - 1) * per_page)
            .fetch_all(&state.pool)
            .await,
        "fetch listing",
    );

    let pagination = Pagination {
        current,
        total_pages,
        prev_url: (current > 1).then(|| page_url(current - 1)),
        next_url: (current < total_pages).then(|| page_url(current + 1)),
    };

    Some((
        format_posts_for_listing(&state.pool, posts).await,
        pagination,
    ))
}

/// Append `page=N` to a URL, omitting it for the first page.
fn with_page(path: &str, page: i64) -> String {
    let separator = if path.contains('?') { '&' } else { '?' };
    if page <= 1 {
        path.to_string()
    } else {
        format!("{path}{separator}page={page}")
    }
}

/// GET / -> Shows home page.
pub async fn home_page(State(state): State<AppState>, Query(query): Query<PageQuery>) -> Response {
    let Some((posts, pagination)) =
        fetch_listing(&state, "", &[], query.page, |p| with_page("/", p)).await
    else {
        return not_found(&state).await;
    };

    let mut context = base_context(&state).await;
    context.insert("posts", &posts);
    context.insert("pagination", &pagination);

    render_theme(&state, &["templates/index.html"], &context, StatusCode::OK).await
}

/// Look up a public post or page by slug.
async fn find_public_post(state: &AppState, slug: &str, is_page: bool) -> Option<Post> {
    or_log(
        sqlx::query_as::<_, Post>(&format!(
            "{POST_SELECT} WHERE slug = ? AND is_page = ? AND {PUBLIC_POST_FILTER}"
        ))
        .bind(slug)
        .bind(is_page)
        .fetch_optional(&state.pool)
        .await,
        "find public post",
    )
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

/// Public path of an post or page, percent-encoded so non-Latin slugs are
/// valid in `Location` headers, sitemaps and feeds.
pub fn post_path(slug: &str, is_page: bool) -> String {
    let slug = url_encode(slug);
    if is_page {
        format!("/{slug}")
    } else {
        format!("/post/{slug}")
    }
}

/// Render an post or page with SEO metadata.
/// Whether a request is a person reading the page, not a bot, a link
/// preview, a browser prefetch, or the site's own admin.
async fn is_countable_view(headers: &HeaderMap, session: &Session) -> bool {
    const BOT_MARKERS: &[&str] = &[
        "bot",
        "crawl",
        "spider",
        "slurp",
        "preview",
        "fetch",
        "curl",
        "wget",
        "python",
        "http",
        "headless",
        "monitor",
        "scan",
        "facebookexternalhit",
        "embedly",
    ];

    let user_agent = headers
        .get(header::USER_AGENT)
        .and_then(|v| v.to_str().ok())
        .unwrap_or_default()
        .to_ascii_lowercase();
    if user_agent.is_empty() || BOT_MARKERS.iter().any(|m| user_agent.contains(m)) {
        return false;
    }

    let prefetch = ["purpose", "sec-purpose"].iter().any(|name| {
        headers
            .get(*name)
            .and_then(|v| v.to_str().ok())
            .is_some_and(|v| v.contains("prefetch") || v.contains("prerender"))
    });
    if prefetch {
        return false;
    }

    let is_admin: Option<bool> = session.get("admin_logged_in").await.unwrap_or(None);
    is_admin != Some(true)
}

/// Host of an external referring site (`news.ycombinator.com`), if any.
fn referrer_host(headers: &HeaderMap, base_url: &str) -> Option<String> {
    let referer = headers.get(header::REFERER)?.to_str().ok()?;
    let host = referer
        .split_once("://")?
        .1
        .split(['/', '?', '#', ':'])
        .next()?
        .trim_start_matches("www.")
        .to_ascii_lowercase();
    let own_host = base_url
        .split_once("://")
        .map_or(base_url, |(_, rest)| rest)
        .split([':', '/'])
        .next()
        .unwrap_or_default()
        .trim_start_matches("www.");
    (!host.is_empty() && host != own_host && host.len() <= 253).then_some(host)
}

/// Count a view in the background so the page isn't delayed.
fn record_view(state: &AppState, post_id: i64, referrer: Option<String>) {
    let pool = state.pool.clone();
    tokio::spawn(async move {
        let _ = sqlx::query("UPDATE posts SET views = views + 1 WHERE id = ?")
            .bind(post_id)
            .execute(&pool)
            .await;
        let _ = sqlx::query(
            "INSERT INTO daily_views (day, post_id, views) VALUES (date('now'), ?, 1)
             ON CONFLICT(day, post_id) DO UPDATE SET views = views + 1",
        )
        .bind(post_id)
        .execute(&pool)
        .await;
        if let Some(host) = referrer {
            let _ = sqlx::query(
                "INSERT INTO daily_referrers (day, host, views) VALUES (date('now'), ?, 1)
                 ON CONFLICT(day, host) DO UPDATE SET views = views + 1",
            )
            .bind(host)
            .execute(&pool)
            .await;
        }
    });
}

async fn render_post(
    state: &AppState,
    mut post: Post,
    headers: &HeaderMap,
    session: &Session,
) -> Response {
    if is_countable_view(headers, session).await {
        record_view(
            state,
            post.id,
            referrer_host(headers, &state.config.base_url),
        );
    }

    let content_html = crate::utils::render_safe_markdown(&post.content);
    let description = excerpt(&post.content, META_DESCRIPTION_CHARS);
    post.reading_time = crate::utils::calculate_reading_time(&post.content);
    post.tags = crate::tags::get_tags_for_post(&state.pool, post.id).await;

    let canonical_url = format!(
        "{}{}",
        state.config.base_url,
        post_path(&post.slug, post.is_page)
    );
    let tag_names: Vec<&str> = post.tags.iter().map(|t| t.name.as_str()).collect();
    let seo_head = seo_head(
        state,
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
                }
            },
            noindex: false,
        },
    )
    .await;

    post.created_at = crate::utils::format_display_date(&post.published_at);

    let mut context = base_context(state).await;
    context.insert("seo_head", &seo_head);
    context.insert("content_html", &content_html);
    context.insert("post", &post);

    let templates: &[&str] = if post.is_page {
        &["templates/page.html", "templates/post.html"]
    } else {
        &["templates/post.html"]
    };
    render_theme(state, templates, &context, StatusCode::OK).await
}

/// GET /post/:slug -> Shows post page.
pub async fn show_post(
    State(state): State<AppState>,
    Path(slug): Path<String>,
    headers: HeaderMap,
    session: Session,
) -> Response {
    if let Some(post) = find_public_post(&state, &slug, false).await {
        return render_post(&state, post, &headers, &session).await;
    }
    match redirect_to_slug(&state, &slug).await {
        Some(redirect) => redirect,
        None => not_found(&state).await,
    }
}

/// GET /article/:slug -> Posts used to live here; send visitors to the new address.
pub async fn legacy_article(State(state): State<AppState>, Path(slug): Path<String>) -> Response {
    match redirect_to_slug(&state, &slug).await {
        Some(redirect) => redirect,
        None => not_found(&state).await,
    }
}

/// GET /:slug -> Shows a standalone page (About, Contact...).
///
/// A post with this slug is redirected to its post URL, which keeps
/// WordPress-style `/post-name` links working after an import.
pub async fn show_page(
    State(state): State<AppState>,
    Path(slug): Path<String>,
    headers: HeaderMap,
    session: Session,
) -> Response {
    if let Some(page) = find_public_post(&state, &slug, true).await {
        return render_post(&state, page, &headers, &session).await;
    }
    match redirect_to_slug(&state, &slug).await {
        Some(redirect) => redirect,
        None => not_found(&state).await,
    }
}

/// Permanent redirect to the public post or page currently or formerly at `slug`.
///
/// Imported slugs are transliterated (`dünya` → `dunya`), so that form is tried too.
async fn redirect_to_slug(state: &AppState, slug: &str) -> Option<Response> {
    let slugified = crate::utils::slugify(slug);
    let candidates = if slugified == slug {
        vec![slug]
    } else {
        vec![slug, slugified.as_str()]
    };

    for candidate in candidates {
        for is_page in [false, true] {
            if let Some(post) = find_public_post(state, candidate, is_page).await {
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

/// Themed 404 page (falls back to plain HTML when the theme has no `404.html`).
pub async fn not_found(state: &AppState) -> Response {
    let mut context = base_context(state).await;
    let blog_name = get_setting(&state.pool, "blog_name", "Bloogla").await;
    let seo_head = seo_head(
        state,
        &SeoMeta {
            title: &blog_name,
            description: "",
            canonical_url: &state.config.base_url,
            image: None,
            kind: SeoKind::Website,
            noindex: true,
        },
    )
    .await;
    context.insert("seo_head", &seo_head);

    render_theme(
        state,
        &["templates/404.html"],
        &context,
        StatusCode::NOT_FOUND,
    )
    .await
}

/// Router fallback for unknown paths.
///
/// Old WordPress permalinks (`/post-name/`, `/2020/05/post-name/`) end with the
/// slug, so the last path segment is tried before giving up.
pub async fn fallback(State(state): State<AppState>, method: Method, uri: Uri) -> Response {
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

/// GET /tag/:slug -> List posts by tag.
pub async fn tag_page(
    State(state): State<AppState>,
    Path(slug): Path<String>,
    Query(query): Query<PageQuery>,
) -> Response {
    let tag = or_log(
        sqlx::query_as::<_, Tag>("SELECT id, name, slug FROM tags WHERE slug = ?")
            .bind(&slug)
            .fetch_optional(&state.pool)
            .await,
        "find tag",
    );
    let Some(tag) = tag else {
        return not_found(&state).await;
    };

    let base_path = format!("/tag/{}", url_encode(&tag.slug));
    let Some((posts, pagination)) = fetch_listing(
        &state,
        "AND id IN (SELECT at.post_id FROM post_tags at
                    JOIN tags t ON t.id = at.tag_id WHERE t.slug = ?)",
        &[&tag.slug],
        query.page,
        |p| with_page(&base_path, p),
    )
    .await
    else {
        return not_found(&state).await;
    };

    let mut context = base_context(&state).await;
    context.insert("tag_name", &tag.name);
    context.insert("posts", &posts);
    context.insert("pagination", &pagination);

    render_theme(&state, &["templates/tag.html"], &context, StatusCode::OK).await
}

/// GET /search?q=query -> Search query.
pub async fn search_post(
    State(state): State<AppState>,
    Query(query): Query<SearchQuery>,
) -> Response {
    let search_term = query.q.unwrap_or_default().trim().to_string();

    if search_term.is_empty() {
        return Redirect::to("/").into_response();
    }

    let base_path = format!("/search?q={}", url_encode(&search_term));
    // Full-text index; a query of only punctuation matches nothing.
    let fts_query = crate::utils::fts_query(&search_term).unwrap_or_else(|| "\"\"".to_string());

    let Some((posts, pagination)) = fetch_listing(
        &state,
        "AND id IN (SELECT rowid FROM posts_fts WHERE posts_fts MATCH ?)",
        &[&fts_query],
        query.page,
        |p| with_page(&base_path, p),
    )
    .await
    else {
        return not_found(&state).await;
    };

    let blog_name = get_setting(&state.pool, "blog_name", "Bloogla").await;
    let seo_head = seo_head(
        &state,
        &SeoMeta {
            title: &blog_name,
            description: "",
            canonical_url: &state.config.base_url,
            image: None,
            kind: SeoKind::Website,
            noindex: true,
        },
    )
    .await;

    let mut context = base_context(&state).await;
    context.insert("seo_head", &seo_head);
    context.insert("posts", &posts);
    context.insert("pagination", &pagination);
    context.insert("search_query", &search_term);

    render_theme(&state, &["templates/index.html"], &context, StatusCode::OK).await
}

/// GET /health -> Show health status.
pub async fn health_check() -> impl IntoResponse {
    StatusCode::OK
}

/// GET /favicon.ico -> The site icon chosen in Settings.
pub async fn favicon(State(state): State<AppState>) -> Response {
    let icon = get_setting(&state.pool, "site_icon", "").await;
    // Redirect needs a valid header value.
    if icon.is_empty() || !icon.is_ascii() {
        return StatusCode::NOT_FOUND.into_response();
    }
    Redirect::temporary(&icon).into_response()
}

/// GET /robots.txt -> Crawler rules pointing at the sitemap.
pub async fn robots_txt(State(state): State<AppState>) -> Response {
    let body = format!(
        "User-agent: *\nDisallow: /admin\nDisallow: /search\n\nSitemap: {}/sitemap.xml\n",
        state.config.base_url
    );
    ([("Content-Type", "text/plain; charset=utf-8")], body).into_response()
}

/// GET /rss.xml -> Dynamic RSS 2.0 Feed output
pub async fn rss_feed(State(state): State<AppState>) -> Response {
    let posts = or_log(
        sqlx::query_as::<_, Post>(&format!(
            "{POST_SELECT} WHERE {LISTED_POST_FILTER} ORDER BY published_at DESC LIMIT 20"
        ))
        .fetch_all(&state.pool)
        .await,
        "rss posts",
    );

    let base_url = &state.config.base_url;

    let mut items = String::new();
    for post in posts {
        let title = xml_escape(&post.title);
        let link = xml_escape(&format!("{base_url}{}", post_path(&post.slug, false)));
        let description = xml_escape(&excerpt(&post.content, 300));
        let pub_date = xml_escape(&to_rfc2822(&post.published_at));
        // Feed readers show the whole post; root-relative links must become absolute.
        let mut html = crate::utils::render_safe_markdown(&post.content)
            .replace("src=\"/", &format!("src=\"{base_url}/"))
            .replace("href=\"/", &format!("href=\"{base_url}/"));
        if let Some(cover) = &post.cover_image {
            let cover = crate::seo::absolute_url(cover, base_url);
            html = format!(r#"<p><img src="{}" alt=""></p>{html}"#, xml_escape(&cover));
        }
        let content = html.replace("]]>", "]]]]><![CDATA[>");

        items.push_str(&format!(
            r#"<item>
                <title>{title}</title>
                <link>{link}</link>
                <guid isPermaLink="true">{link}</guid>
                <description>{description}</description>
                <content:encoded><![CDATA[{content}]]></content:encoded>
                <pubDate>{pub_date}</pubDate>
            </item>"#
        ));
    }

    let raw_blog_name = get_setting(&state.pool, "blog_name", "Bloogla").await;
    let blog_name = xml_escape(&raw_blog_name);
    let raw_description = get_setting(&state.pool, "blog_description", "").await;
    let description = if raw_description.is_empty() {
        format!("Latest posts from {blog_name}")
    } else {
        xml_escape(&raw_description)
    };

    let xml = format!(
        r#"<?xml version="1.0" encoding="UTF-8" ?>
        <rss version="2.0" xmlns:content="http://purl.org/rss/1.0/modules/content/" xmlns:atom="http://www.w3.org/2005/Atom">
            <channel>
                <title>{blog_name}</title>
                <link>{base_url}</link>
                <atom:link href="{base_url}/rss.xml" rel="self" type="application/rss+xml" />
                <description>{description}</description>
                {items}
            </channel>
        </rss>"#
    );

    ([("Content-Type", "application/xml; charset=utf-8")], xml).into_response()
}

/// GET /sitemap.xml -> Dynamic Google Sitemap output
pub async fn sitemap_xml(State(state): State<AppState>) -> Response {
    let posts = or_log(
        sqlx::query_as::<_, Post>(&format!(
            "{POST_SELECT} WHERE {PUBLIC_POST_FILTER} ORDER BY published_at DESC"
        ))
        .fetch_all(&state.pool)
        .await,
        "sitemap posts",
    );

    let base_url = &state.config.base_url;

    let mut urls = format!(
        r#"<url>
            <loc>{}</loc>
            <priority>1.0</priority>
        </url>"#,
        base_url
    );

    for post in posts {
        let loc = xml_escape(&format!(
            "{base_url}{}",
            post_path(&post.slug, post.is_page)
        ));
        let lastmod = post.published_at.get(..10).unwrap_or_default();
        urls.push_str(&format!(
            r#"<url>
                <loc>{loc}</loc>
                <lastmod>{lastmod}</lastmod>
                <priority>0.8</priority>
            </url>"#
        ));
    }

    let xml = format!(
        r#"<?xml version="1.0" encoding="UTF-8"?>
        <urlset xmlns="http://www.sitemaps.org/schemas/sitemap/0.9">
            {}
        </urlset>"#,
        urls
    );

    ([("Content-Type", "application/xml; charset=utf-8")], xml).into_response()
}

/// Add an `ETag` to public HTML and XML responses and answer `304 Not Modified`
/// when the browser or feed reader already has the same version.
pub async fn etag_middleware(
    req: axum::extract::Request,
    next: axum::middleware::Next,
) -> Response {
    use std::hash::{Hash, Hasher};

    let path = req.uri().path();
    let eligible =
        req.method() == Method::GET && !path.starts_with("/admin") && !path.starts_with("/setup");
    let if_none_match = req.headers().get(header::IF_NONE_MATCH).cloned();

    let response = next.run(req).await;
    let is_page = response
        .headers()
        .get(header::CONTENT_TYPE)
        .and_then(|v| v.to_str().ok())
        .is_some_and(|ct| ct.starts_with("text/html") || ct.contains("xml"));
    if !eligible || response.status() != StatusCode::OK || !is_page {
        return response;
    }

    let (mut parts, body) = response.into_parts();
    let Ok(bytes) = axum::body::to_bytes(body, 16 * 1024 * 1024).await else {
        return StatusCode::INTERNAL_SERVER_ERROR.into_response();
    };

    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    bytes.hash(&mut hasher);
    let etag = format!("W/\"{:016x}\"", hasher.finish());

    if let Ok(value) = etag.parse() {
        parts.headers.insert(header::ETAG, value);
    }
    // Always revalidate, so new posts show up immediately.
    parts.headers.insert(
        header::CACHE_CONTROL,
        axum::http::HeaderValue::from_static("no-cache"),
    );

    if if_none_match.is_some_and(|v| v.as_bytes() == etag.as_bytes()) {
        parts.status = StatusCode::NOT_MODIFIED;
        parts.headers.remove(header::CONTENT_LENGTH);
        return Response::from_parts(parts, axum::body::Body::empty());
    }
    Response::from_parts(parts, axum::body::Body::from(bytes))
}

#[cfg(test)]
mod tests {
    use super::referrer_host;
    use axum::http::{header, HeaderMap, HeaderValue};

    fn with_referer(value: &str) -> HeaderMap {
        let mut headers = HeaderMap::new();
        headers.insert(header::REFERER, HeaderValue::from_str(value).unwrap());
        headers
    }

    #[test]
    fn referrer_host_ignores_own_site() {
        let base = "https://dincertekin.com";
        assert_eq!(
            referrer_host(
                &with_referer("https://news.ycombinator.com/item?id=1"),
                base
            ),
            Some("news.ycombinator.com".into())
        );
        assert_eq!(
            referrer_host(&with_referer("https://www.google.com/"), base),
            Some("google.com".into())
        );
        assert_eq!(
            referrer_host(&with_referer("https://www.dincertekin.com/post/a"), base),
            None
        );
        assert_eq!(referrer_host(&HeaderMap::new(), base), None);
    }
}
