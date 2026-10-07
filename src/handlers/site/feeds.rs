//! Machine-readable addresses: RSS feed, sitemap, robots.txt, favicon and
//! the health check.

use crate::app::models::Post;
use crate::app::state::AppState;
use crate::content::markdown::excerpt;
use crate::content::text::{escape_html, to_rfc2822};
use crate::db::posts::{LISTED_POST_FILTER, POST_SELECT, PUBLIC_POST_FILTER};
use crate::db::{or_log, settings};
use crate::server::routes::post_path;

use axum::extract::State;
use axum::http::{header, StatusCode};
use axum::response::{IntoResponse, Redirect, Response};

const XML: [(header::HeaderName, &str); 1] =
    [(header::CONTENT_TYPE, "application/xml; charset=utf-8")];

/// Posts in the RSS feed.
const FEED_SIZE: i64 = 20;

/// GET /rss.xml -> RSS 2.0 feed with the full text of the newest posts.
pub async fn rss(State(state): State<AppState>) -> Response {
    let posts = or_log(
        sqlx::query_as::<_, Post>(&format!(
            "{POST_SELECT} WHERE {LISTED_POST_FILTER} ORDER BY published_at DESC LIMIT ?"
        ))
        .bind(FEED_SIZE)
        .fetch_all(&state.pool)
        .await,
        "rss posts",
    );

    let site = settings::load(&state.pool).await;
    let base_url = &state.config.base_url;

    let mut items = String::new();
    for post in posts {
        let title = escape_html(&post.title);
        let link = escape_html(&format!("{base_url}{}", post_path(&post.slug, false)));
        let description = escape_html(&excerpt(&post.content, 300));
        let pub_date = escape_html(&to_rfc2822(&post.published_at));
        // Feed readers show the whole post; root-relative links must become absolute.
        let mut html = crate::content::absolute_links(
            &crate::content::render(&state, &post.content).await,
            base_url,
        );
        if let Some(cover) = &post.cover_image {
            let cover = crate::content::seo::absolute_url(cover, base_url);
            html = format!(r#"<p><img src="{}" alt=""></p>{html}"#, escape_html(&cover));
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

    let blog_name = escape_html(&site.blog_name);
    let description = if site.blog_description.is_empty() {
        escape_html(
            &site
                .language
                .tv("Latest posts from {site}", &site.blog_name),
        )
    } else {
        escape_html(&site.blog_description)
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

    (XML, xml).into_response()
}

/// GET /sitemap.xml -> Every public post and page, for search engines.
pub async fn sitemap(State(state): State<AppState>) -> Response {
    let rows: Vec<(String, bool, String)> = or_log(
        sqlx::query_as(&format!(
            "SELECT slug, is_page, published_at FROM posts
             WHERE {PUBLIC_POST_FILTER} ORDER BY published_at DESC"
        ))
        .fetch_all(&state.pool)
        .await,
        "sitemap posts",
    );

    let base_url = &state.config.base_url;
    let mut urls = format!(
        r#"<url>
            <loc>{base_url}</loc>
            <priority>1.0</priority>
        </url>"#
    );
    for (slug, is_page, published_at) in rows {
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

/// GET /favicon.ico -> The site icon chosen in Settings.
pub async fn favicon(State(state): State<AppState>) -> Response {
    let site = settings::load(&state.pool).await;
    // No icon: "no content" rather than "not found", which browsers log as an
    // error on every page. (A redirect also needs a valid header value.)
    if site.site_icon.is_empty() || !site.site_icon.is_ascii() {
        return StatusCode::NO_CONTENT.into_response();
    }
    Redirect::temporary(&site.site_icon).into_response()
}

/// GET /health -> 200 OK while the server is running (for uptime monitors).
pub async fn health() -> StatusCode {
    StatusCode::OK
}
