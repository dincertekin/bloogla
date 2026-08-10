use axum::{
    extract::{Path, State},
    response::{Html, IntoResponse, Response},
};
use comrak::{markdown_to_html, ComrakOptions};

use crate::models::Post;
use crate::templates::{IndexTemplate, PostTemplate, TagPageTemplate};
use crate::AppState;

fn xml_escape(input: &str) -> String {
    input
        .replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&apos;")
}

async fn format_posts_for_listing(pool: &sqlx::Pool<sqlx::Sqlite>, posts: Vec<Post>) -> Vec<Post> {
    let mut options = ComrakOptions::default();
    options.render.unsafe_ = true;

    let mut formatted: Vec<Post> = posts
        .into_iter()
        .map(|mut post| {
            post.reading_time = crate::utils::calculate_reading_time(&post.content);

            let raw_html = markdown_to_html(&post.content, &options);

            let plain_text = ammonia::Builder::new()
                .tags(std::collections::HashSet::new())
                .clean(&raw_html)
                .to_string();

            let snippet = if plain_text.chars().count() > 180 {
                format!("{}...", plain_text.chars().take(180).collect::<String>())
            } else {
                plain_text
            };

            post.content = snippet;
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

pub async fn home_page(State(state): State<AppState>) -> impl IntoResponse {
    let posts = sqlx::query_as::<_, Post>(
        "SELECT id, title, slug, content, cover_image, COALESCE(views, 0) as views, COALESCE(created_at, CURRENT_TIMESTAMP) as created_at FROM posts ORDER BY id DESC"
    )
    .fetch_all(&state.pool)
    .await
    .unwrap_or_default();

    let formatted_posts = format_posts_for_listing(&state.pool, posts).await;

    IndexTemplate {
        blog_name: state.config.blog_name,
        posts: formatted_posts,
    }
}

pub async fn show_post(
    State(state): State<AppState>,
    Path(slug): Path<String>,
) -> impl IntoResponse {
    let _ = sqlx::query("UPDATE posts SET views = views + 1 WHERE slug = ?")
        .bind(&slug)
        .execute(&state.pool)
        .await;

    let post = sqlx::query_as::<_, Post>(
        "SELECT id, title, slug, content, cover_image, COALESCE(views, 0) as views, COALESCE(created_at, CURRENT_TIMESTAMP) as created_at FROM posts WHERE slug = ?"
    )
    .bind(&slug)
    .fetch_optional(&state.pool)
    .await
    .unwrap_or(None);

    match post {
        Some(mut p) => {
            let content_html = crate::utils::render_safe_markdown(&p.content);
            p.reading_time = crate::utils::calculate_reading_time(&p.content);
            p.tags = crate::tags::get_tags_for_post(&state.pool, p.id).await;

            PostTemplate {
                blog_name: state.config.blog_name,
                content_html,
                post: p,
            }
            .into_response()
        }
        None => Html("<h1>404 Not Found</h1>".to_string()).into_response(),
    }
}

/// GET /tag/:slug -> List posts with specified tag.
pub async fn tag_page(
    State(state): State<AppState>,
    Path(slug): Path<String>,
) -> impl IntoResponse {
    let tag =
        sqlx::query_as::<_, crate::models::Tag>("SELECT id, name, slug FROM tags WHERE slug = ?")
            .bind(&slug)
            .fetch_optional(&state.pool)
            .await
            .unwrap_or(None);

    let tag = match tag {
        Some(t) => t,
        None => return Html("<h1>404 Not Found</h1>".to_string()).into_response(),
    };

    let posts = sqlx::query_as::<_, Post>(
        "SELECT p.id, p.title, p.slug, p.content, p.cover_image, COALESCE(p.views, 0) as views, COALESCE(p.created_at, CURRENT_TIMESTAMP) as created_at
         FROM posts p
         INNER JOIN post_tags pt ON pt.post_id = p.id
         WHERE pt.tag_id = ?
         ORDER BY p.id DESC"
    )
    .bind(tag.id)
    .fetch_all(&state.pool)
    .await
    .unwrap_or_default();

    let formatted_posts = format_posts_for_listing(&state.pool, posts).await;

    TagPageTemplate {
        blog_name: state.config.blog_name,
        tag_name: tag.name,
        posts: formatted_posts,
    }
    .into_response()
}

// GET /health -> Show health status.
pub async fn health_check() -> impl IntoResponse {
    axum::http::StatusCode::OK
}

/// GET /rss.xml -> Dynamic RSS 2.0 Feed output
pub async fn rss_feed(State(state): State<AppState>) -> Response {
    let posts = sqlx::query_as::<_, Post>(
        "SELECT id, title, slug, content, cover_image, COALESCE(views, 0) as views, COALESCE(created_at, CURRENT_TIMESTAMP) as created_at FROM posts ORDER BY id DESC LIMIT 20"
    )
    .fetch_all(&state.pool)
    .await
    .unwrap_or_default();

    let base_url = &state.config.base_url;

    let mut items = String::new();
    for post in posts {
        let title = xml_escape(&post.title);
        let slug = xml_escape(&post.slug);
        let pub_date = xml_escape(&post.created_at);

        items.push_str(&format!(
            r#"<item>
                <title>{}</title>
                <link>{}/post/{}</link>
                <guid>{}/post/{}</guid>
                <pubDate>{}</pubDate>
            </item>"#,
            title, base_url, slug, base_url, slug, pub_date
        ));
    }

    let blog_name = xml_escape(&state.config.blog_name);

    let xml = format!(
        r#"<?xml version="1.0" encoding="UTF-8" ?>
        <rss version="2.0">
            <channel>
                <title>{}</title>
                <link>{}</link>
                <description>Latest articles from {}</description>
                {}
            </channel>
        </rss>"#,
        blog_name, base_url, blog_name, items
    );

    Response::builder()
        .header("Content-Type", "application/xml; charset=utf-8")
        .body(xml.into())
        .unwrap()
}

/// GET /sitemap.xml -> Dynamic Google Sitemap output
pub async fn sitemap_xml(State(state): State<AppState>) -> Response {
    let posts = sqlx::query_as::<_, Post>(
        "SELECT id, title, slug, content, cover_image, COALESCE(views, 0) as views, COALESCE(created_at, CURRENT_TIMESTAMP) as created_at FROM posts ORDER BY id DESC"
    )
    .fetch_all(&state.pool)
    .await
    .unwrap_or_default();

    let base_url = &state.config.base_url;

    let mut urls = format!(
        r#"<url>
            <loc>{}</loc>
            <priority>1.0</priority>
        </url>"#,
        base_url
    );

    for post in posts {
        let slug = xml_escape(&post.slug);
        urls.push_str(&format!(
            r#"<url>
                <loc>{}/post/{}</loc>
                <priority>0.8</priority>
            </url>"#,
            base_url, slug
        ));
    }

    let xml = format!(
        r#"<?xml version="1.0" encoding="UTF-8"?>
        <urlset xmlns="http://www.sitemaps.org/schemas/sitemap/0.9">
            {}
        </urlset>"#,
        urls
    );

    Response::builder()
        .header("Content-Type", "application/xml; charset=utf-8")
        .body(xml.into())
        .unwrap()
}
