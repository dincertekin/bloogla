//! Machine-readable addresses: RSS feed, sitemap, robots.txt, favicon and
//! the health check.

use crate::app::models::Post;
use crate::app::state::AppState;
use crate::content::markdown::excerpt;
use crate::content::text::{escape_html, to_rfc2822, url_encode};
use crate::db::{posts, settings, tags};
use crate::handlers::site::not_found;
use crate::server::routes::post_path;

use axum::extract::{Path, State};
use axum::http::{header, StatusCode};
use axum::response::{IntoResponse, Redirect, Response};

const XML: [(header::HeaderName, &str); 1] =
    [(header::CONTENT_TYPE, "application/xml; charset=utf-8")];

/// Posts in the RSS feed.
const FEED_SIZE: i64 = 20;

/// GET /rss.xml -> RSS 2.0 feed with the full text of the newest posts.
pub async fn rss(State(state): State<AppState>) -> Response {
    let posts = posts::newest_listed(&state.pool, FEED_SIZE).await;
    let site = settings::load(&state.pool).await;
    let description = if site.blog_description.is_empty() {
        site.language
            .tv("Latest posts from {site}", &site.blog_name)
    } else {
        site.blog_description.clone()
    };
    let xml = feed(&state, posts, &site.blog_name, &description, "/rss.xml").await;
    (XML, xml).into_response()
}

/// GET /tag/:slug/rss.xml -> The same feed with only the posts of one tag,
/// for readers who follow a single topic.
pub async fn tag_rss(State(state): State<AppState>, Path(slug): Path<String>) -> Response {
    let Some(tag) = tags::find_listed_by_slug(&state.pool, &slug).await else {
        return not_found(&state).await;
    };
    let posts = posts::newest_listed_with_tag(&state.pool, tag.id, FEED_SIZE).await;
    let site = settings::load(&state.pool).await;
    let title = format!("{} · {}", tag.name, site.blog_name);
    let description = site
        .language
        .tv2("Posts about {tag} on {site}.", &tag.name, &site.blog_name);
    let xml = feed(
        &state,
        posts,
        &title,
        &description,
        &tag_feed_path(&tag.slug),
    )
    .await;
    (XML, xml).into_response()
}

/// Address of a tag's feed, percent-encoded like the tag page's.
pub fn tag_feed_path(slug: &str) -> String {
    format!("/tag/{}/rss.xml", url_encode(slug))
}

/// An RSS 2.0 document for `posts`. `self_path` is the feed's own address.
async fn feed(
    state: &AppState,
    posts: Vec<Post>,
    title: &str,
    description: &str,
    self_path: &str,
) -> String {
    let base_url = &state.config.base_url;

    let mut items = String::new();
    for post in posts {
        let item_title = escape_html(&post.title);
        let link = escape_html(&format!("{base_url}{}", post_path(&post.slug, false)));
        let summary = escape_html(&excerpt(&post.content, 300));
        let pub_date = escape_html(&to_rfc2822(&post.published_at));
        // Feed readers show the whole post; root-relative links must become absolute.
        let mut html = crate::content::absolute_links(
            &crate::content::render(state, &post.content).await,
            base_url,
        );
        if let Some(cover) = &post.cover_image {
            let cover = crate::content::seo::absolute_url(cover, base_url);
            html = format!(r#"<p><img src="{}" alt=""></p>{html}"#, escape_html(&cover));
        }
        let content = html.replace("]]>", "]]]]><![CDATA[>");

        items.push_str(&format!(
            r#"<item>
                <title>{item_title}</title>
                <link>{link}</link>
                <guid isPermaLink="true">{link}</guid>
                <description>{summary}</description>
                <content:encoded><![CDATA[{content}]]></content:encoded>
                <pubDate>{pub_date}</pubDate>
            </item>"#
        ));
    }

    let title = escape_html(title);
    let description = escape_html(description);
    let self_url = escape_html(&format!("{base_url}{self_path}"));
    format!(
        r#"<?xml version="1.0" encoding="UTF-8" ?>
        <rss version="2.0" xmlns:content="http://purl.org/rss/1.0/modules/content/" xmlns:atom="http://www.w3.org/2005/Atom">
            <channel>
                <title>{title}</title>
                <link>{base_url}</link>
                <atom:link href="{self_url}" rel="self" type="application/rss+xml" />
                <description>{description}</description>
                {items}
            </channel>
        </rss>"#
    )
}

/// GET /sitemap.xml -> Every public post and page, for search engines.
pub async fn sitemap(State(state): State<AppState>) -> Response {
    let rows = posts::for_sitemap(&state.pool).await;

    let base_url = &state.config.base_url;
    let mut urls = format!(
        r#"<url>
            <loc>{base_url}</loc>
            <priority>1.0</priority>
        </url>"#
    );
    // A home page is listed as the site itself (above), and the posts
    // have their own list at /blog.
    let home_page = settings::load(&state.pool)
        .await
        .home_page
        .filter(|home| rows.iter().any(|(id, ..)| id == home));
    if home_page.is_some() {
        urls.push_str(&format!(
            r#"<url>
                <loc>{base_url}/blog</loc>
                <priority>0.9</priority>
            </url>"#
        ));
    }
    for (id, slug, is_page, published_at) in rows {
        if home_page == Some(id) {
            continue;
        }
        let loc = escape_html(&format!("{base_url}{}", post_path(&slug, is_page)));
        let lastmod = published_at.get(..10).unwrap_or_default();
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
            {urls}
        </urlset>"#
    );
    (XML, xml).into_response()
}

/// GET /robots.txt -> Crawler rules pointing at the sitemap.
pub async fn robots_txt(State(state): State<AppState>) -> Response {
    let body = format!(
        "User-agent: *\nDisallow: /admin\nDisallow: /search\n\nSitemap: {}/sitemap.xml\n",
        state.config.base_url
    );
    ([(header::CONTENT_TYPE, "text/plain; charset=utf-8")], body).into_response()
}

/// GET /favicon.ico -> The site icon chosen in Settings, or Bloogla's logo.
pub async fn favicon(State(state): State<AppState>) -> Response {
    let site = settings::load(&state.pool).await;
    // A redirect needs a valid header value, so an unusual address gets the logo too.
    if site.site_icon.is_empty() || !site.site_icon.is_ascii() {
        return Redirect::temporary(crate::content::seo::DEFAULT_ICON_PNG).into_response();
    }
    Redirect::temporary(&site.site_icon).into_response()
}

/// GET /theme-assets/:theme/static/* when the theme hasn't got that file:
/// the shared copy built into Bloogla (`themes/_shared/static/`), if any.
pub async fn shared_theme_file(uri: axum::http::Uri) -> Response {
    // The path inside `/theme-assets`: `/<theme>/static/<file>`.
    let file = uri
        .path()
        .trim_start_matches('/')
        .split_once('/')
        .and_then(|(_, rest)| rest.strip_prefix("static/"));
    let Some((bytes, kind)) = file.and_then(crate::services::themes::shared_static) else {
        return StatusCode::NOT_FOUND.into_response();
    };
    ([(header::CONTENT_TYPE, kind)], bytes).into_response()
}

/// GET /health -> 200 OK while the server is running (for uptime monitors).
pub async fn health() -> StatusCode {
    StatusCode::OK
}
