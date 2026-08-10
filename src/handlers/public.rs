use axum::{
    extract::{Path, Query, State},
    response::{Html, IntoResponse, Response},
};
use comrak::{markdown_to_html, ComrakOptions};

use crate::models::{Article, SearchQuery, Tag};
use crate::templates::{ArticleTemplate, IndexTemplate, TagPageTemplate};
use crate::AppState;

fn xml_escape(input: &str) -> String {
    input
        .replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&apos;")
}

async fn format_articles_for_listing(
    pool: &sqlx::Pool<sqlx::Sqlite>,
    articles: Vec<Article>,
) -> Vec<Article> {
    let mut options = ComrakOptions::default();
    options.render.unsafe_ = true;

    let mut formatted: Vec<Article> = articles
        .into_iter()
        .map(|mut article| {
            article.reading_time = crate::utils::calculate_reading_time(&article.content);
            article.created_at = crate::utils::format_display_date(&article.created_at);

            let raw_html = markdown_to_html(&article.content, &options);

            let plain_text = ammonia::Builder::new()
                .tags(std::collections::HashSet::new())
                .clean(&raw_html)
                .to_string();

            let snippet = if plain_text.chars().count() > 180 {
                format!("{}...", plain_text.chars().take(180).collect::<String>())
            } else {
                plain_text
            };

            article.content = snippet;
            article
        })
        .collect();

    let article_ids: Vec<i64> = formatted.iter().map(|p| p.id).collect();
    let tag_map = crate::tags::get_tags_for_articles(pool, &article_ids).await;
    for article in formatted.iter_mut() {
        article.tags = tag_map.get(&article.id).cloned().unwrap_or_default();
    }

    formatted
}

pub async fn home_page(State(state): State<AppState>) -> impl IntoResponse {
    let articles = sqlx::query_as::<_, Article>(
        "SELECT id, title, slug, content, cover_image, COALESCE(views, 0) as views, COALESCE(created_at, CURRENT_TIMESTAMP) as created_at FROM articles ORDER BY id DESC"
    )
    .fetch_all(&state.pool)
    .await
    .unwrap_or_default();

    let tags = sqlx::query_as::<_, Tag>("SELECT id, name, slug FROM tags ORDER BY name ASC")
        .fetch_all(&state.pool)
        .await
        .unwrap_or_default();

    let formatted_articles = format_articles_for_listing(&state.pool, articles).await;

    IndexTemplate {
        blog_name: state.config.blog_name,
        articles: formatted_articles,
        tags,
        search_query: String::new(),
    }
}

pub async fn show_article(
    State(state): State<AppState>,
    Path(slug): Path<String>,
) -> impl IntoResponse {
    let _ = sqlx::query("UPDATE articles SET views = views + 1 WHERE slug = ?")
        .bind(&slug)
        .execute(&state.pool)
        .await;

    let article = sqlx::query_as::<_, Article>(
        "SELECT id, title, slug, content, cover_image, COALESCE(views, 0) as views, COALESCE(created_at, CURRENT_TIMESTAMP) as created_at FROM articles WHERE slug = ?"
    )
    .bind(&slug)
    .fetch_optional(&state.pool)
    .await
    .unwrap_or(None);

    let tags = sqlx::query_as::<_, Tag>("SELECT id, name, slug FROM tags ORDER BY name ASC")
        .fetch_all(&state.pool)
        .await
        .unwrap_or_default();

    match article {
        Some(mut p) => {
            let content_html = crate::utils::render_safe_markdown(&p.content);
            p.reading_time = crate::utils::calculate_reading_time(&p.content);
            p.created_at = crate::utils::format_display_date(&p.created_at);
            p.tags = crate::tags::get_tags_for_article(&state.pool, p.id).await;

            ArticleTemplate {
                blog_name: state.config.blog_name,
                content_html,
                article: p,
                tags,
                search_query: String::new(),
            }
            .into_response()
        }
        None => Html("<h1>404 Not Found</h1>".to_string()).into_response(),
    }
}

/// GET /tag/:slug -> List articles by tag.
pub async fn tag_page(
    State(state): State<AppState>,
    Path(slug): Path<String>,
) -> impl IntoResponse {
    let tag =
        sqlx::query_as::<_, crate::models::Tag>("SELECT id, name, slug FROM tags WHERE slug = ?")
            .bind(&slug)
            .fetch_optional(&state.pool)
            .await
            .ok()
            .flatten();

    let tag = match tag {
        Some(t) => t,
        None => return Html("<h1>404 Not Found</h1>".to_string()).into_response(),
    };

    let all_tags = sqlx::query_as::<_, crate::models::Tag>(
        "SELECT id, name, slug FROM tags ORDER BY name ASC",
    )
    .fetch_all(&state.pool)
    .await
    .unwrap_or_default();

    let articles = sqlx::query_as::<_, Article>(
        "SELECT p.id, p.title, p.slug, p.content, p.cover_image, COALESCE(p.views, 0) as views, COALESCE(p.created_at, CURRENT_TIMESTAMP) as created_at
         FROM articles p
         INNER JOIN article_tags pt ON pt.article_id = p.id
         WHERE pt.tag_id = ?
         ORDER BY p.id DESC"
    )
    .bind(tag.id)
    .fetch_all(&state.pool)
    .await
    .unwrap_or_default();

    let formatted_articles = format_articles_for_listing(&state.pool, articles).await;

    TagPageTemplate {
        blog_name: state.config.blog_name,
        tag_name: tag.name,
        articles: formatted_articles,
        tags: all_tags,
        search_query: String::new(),
    }
    .into_response()
}

// GET /health -> Show health status.
pub async fn health_check() -> impl IntoResponse {
    axum::http::StatusCode::OK
}

/// GET /rss.xml -> Dynamic RSS 2.0 Feed output
pub async fn rss_feed(State(state): State<AppState>) -> Response {
    let articles = sqlx::query_as::<_, Article>(
        "SELECT id, title, slug, content, cover_image, COALESCE(views, 0) as views, COALESCE(created_at, CURRENT_TIMESTAMP) as created_at FROM articles ORDER BY id DESC LIMIT 20"
    )
    .fetch_all(&state.pool)
    .await
    .unwrap_or_default();

    let base_url = &state.config.base_url;

    let mut items = String::new();
    for article in articles {
        let title = xml_escape(&article.title);
        let slug = xml_escape(&article.slug);
        let pub_date = xml_escape(&article.created_at);

        items.push_str(&format!(
            r#"<item>
                <title>{}</title>
                <link>{}/article/{}</link>
                <guid>{}/article/{}</guid>
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
    let articles = sqlx::query_as::<_, Article>(
        "SELECT id, title, slug, content, cover_image, COALESCE(views, 0) as views, COALESCE(created_at, CURRENT_TIMESTAMP) as created_at FROM articles ORDER BY id DESC"
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

    for article in articles {
        let slug = xml_escape(&article.slug);
        urls.push_str(&format!(
            r#"<url>
                <loc>{}/article/{}</loc>
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

/// GET /search?q=query -> Search query.
pub async fn search_article(
    State(state): State<AppState>,
    Query(query): Query<SearchQuery>,
) -> impl IntoResponse {
    let search_term = query.q.unwrap_or_default().trim().to_string();

    if search_term.is_empty() {
        return axum::response::Redirect::to("/").into_response();
    }

    let pattern = format!("%{}%", search_term);
    let articles = sqlx::query_as::<_, Article>(
        "SELECT id, title, slug, content, cover_image, COALESCE(views, 0) as views, COALESCE(created_at, CURRENT_TIMESTAMP) as created_at
         FROM articles
         WHERE title LIKE ? OR content LIKE ?
         ORDER BY id DESC"
    )
    .bind(&pattern)
    .bind(&pattern)
    .fetch_all(&state.pool)
    .await
    .unwrap_or_default();

    let tags = sqlx::query_as::<_, Tag>("SELECT id, name, slug FROM tags ORDER BY name ASC")
        .fetch_all(&state.pool)
        .await
        .unwrap_or_default();

    let formatted_articles = format_articles_for_listing(&state.pool, articles).await;

    IndexTemplate {
        blog_name: state.config.blog_name,
        articles: formatted_articles,
        tags,
        search_query: search_term,
    }
    .into_response()
}
