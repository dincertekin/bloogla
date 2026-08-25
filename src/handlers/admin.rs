use axum::extract::{Path, State};
use axum::response::{Html, IntoResponse, Redirect};
use axum_extra::extract::Form;
use tower_sessions::Session;

use crate::models::{AppState, Article, CreateArticleForm, CreateTagForm, Tag};
use crate::templates::{
    AdminTemplate, ArticleItemTemplate, ArticlesTemplate, EditArticleTemplate, TagItemTemplate,
    TagsTemplate,
};

fn slugify(title: &str) -> String {
    let mut slug = String::new();
    let mut last_was_dash = false;

    for c in title.to_lowercase().chars() {
        if c.is_ascii_alphanumeric() {
            slug.push(c);
            last_was_dash = false;
        } else if !last_was_dash {
            slug.push('-');
            last_was_dash = true;
        }
    }

    let trimmed = slug.trim_matches('-').to_string();
    if trimmed.is_empty() {
        "article".to_string()
    } else {
        trimmed
    }
}

async fn generate_unique_slug(
    pool: &sqlx::Pool<sqlx::Sqlite>,
    title: &str,
    exclude_id: Option<i64>,
) -> String {
    let base = slugify(title);
    let mut candidate = base.clone();
    let mut counter = 2;

    loop {
        let exists: Option<i64> = sqlx::query_scalar(
            "SELECT id FROM articles WHERE slug = ? AND (?2 IS NULL OR id != ?2)",
        )
        .bind(&candidate)
        .bind(exclude_id)
        .fetch_optional(pool)
        .await
        .unwrap_or(None);

        match exists {
            None => return candidate,
            Some(_) => {
                candidate = format!("{}-{}", base, counter);
                counter += 1;
            }
        }
    }
}

/// GET /admin -> Show admin panel/dashboard.
pub async fn admin_dashboard(State(state): State<AppState>, session: Session) -> impl IntoResponse {
    let logged_in: Option<bool> = session.get("admin_logged_in").await.unwrap_or(None);
    if logged_in != Some(true) {
        return Redirect::to("/admin/login").into_response();
    }

    let articles = sqlx::query_as::<_, Article>(
        "SELECT id, title, slug, content, cover_image, COALESCE(views, 0) as views, COALESCE(created_at, CURRENT_TIMESTAMP) as created_at FROM articles ORDER BY id DESC"
    )
    .fetch_all(&state.pool)
    .await
    .unwrap_or_default();

    let total_articles = articles.len() as i64;
    let total_views: i64 = articles.iter().map(|p| p.views).sum();

    let mut top_articles = articles.clone();
    top_articles.sort_by(|a, b| b.views.cmp(&a.views));
    top_articles.truncate(5);

    let max_views = top_articles.first().map(|p| p.views).unwrap_or(0).max(1);

    let daily_counts: Vec<(String, i64)> = sqlx::query_as(
        "SELECT date(created_at) as day, COUNT(*) as count FROM articles GROUP BY day ORDER BY day ASC"
    )
    .fetch_all(&state.pool)
    .await
    .unwrap_or_default();

    let mut cumulative = 0i64;
    let growth_series: Vec<(String, i64)> = daily_counts
        .into_iter()
        .map(|(day, count)| {
            cumulative += count;
            (day, cumulative)
        })
        .collect();

    let growth_max = growth_series.last().map(|(_, v)| *v).unwrap_or(0).max(1);
    let growth_first_date = growth_series
        .first()
        .map(|(d, _)| d.clone())
        .unwrap_or_default();
    let growth_last_date = growth_series
        .last()
        .map(|(d, _)| d.clone())
        .unwrap_or_default();

    const CHART_W: f64 = 600.0;
    const CHART_H: f64 = 180.0;
    const PAD: f64 = 20.0;
    let n = growth_series.len();

    let growth_points = if n == 0 {
        String::new()
    } else if n == 1 {
        let y = CHART_H - PAD;
        format!("{:.1},{:.1} {:.1},{:.1}", PAD, y, CHART_W - PAD, y)
    } else {
        growth_series
            .iter()
            .enumerate()
            .map(|(i, (_, value))| {
                let x = PAD + (i as f64 / (n as f64 - 1.0)) * (CHART_W - 2.0 * PAD);
                let y = CHART_H - PAD - (*value as f64 / growth_max as f64) * (CHART_H - 2.0 * PAD);
                format!("{:.1},{:.1}", x, y)
            })
            .collect::<Vec<_>>()
            .join(" ")
    };

    AdminTemplate {
        blog_name: state.config.blog_name,
        active_page: "dashboard",
        total_articles,
        total_views,
        articles,
        top_articles,
        max_views,
        growth_points,
        growth_max,
        growth_first_date,
        growth_last_date,
    }
    .into_response()
}

/// GET /admin/articles -> List all articles.
pub async fn list_articles(State(state): State<AppState>, session: Session) -> impl IntoResponse {
    let logged_in: Option<bool> = session.get("admin_logged_in").await.unwrap_or(None);
    if logged_in != Some(true) {
        return Redirect::to("/admin/login").into_response();
    }

    let mut articles = sqlx::query_as::<_, Article>(
        "SELECT id, title, slug, content, cover_image, COALESCE(views, 0) as views, COALESCE(created_at, CURRENT_TIMESTAMP) as created_at FROM articles ORDER BY id DESC"
    )
    .fetch_all(&state.pool)
    .await
    .unwrap_or_default();

    let article_ids: Vec<i64> = articles.iter().map(|p| p.id).collect();
    let tag_map = crate::tags::get_tags_for_articles(&state.pool, &article_ids).await;
    for article in articles.iter_mut() {
        article.tags = tag_map.get(&article.id).cloned().unwrap_or_default();
    }

    let all_tags = crate::tags::get_all_tags(&state.pool).await;

    ArticlesTemplate {
        blog_name: state.config.blog_name,
        active_page: "articles",
        articles,
        all_tags,
    }
    .into_response()
}

/// POST /admin/articles/new -> Create new article.
pub async fn create_article(
    State(state): State<AppState>,
    session: Session,
    Form(form): Form<CreateArticleForm>,
) -> impl IntoResponse {
    let logged_in: Option<bool> = session.get("admin_logged_in").await.unwrap_or(None);
    if logged_in != Some(true) {
        return Redirect::to("/admin/login").into_response();
    }

    let slug = generate_unique_slug(&state.pool, &form.title, None).await;

    let cover = match form.cover_image {
        Some(ref url) if url.trim().is_empty() => None,
        other => other,
    };

    let result =
        sqlx::query("INSERT INTO articles (title, slug, content, cover_image) VALUES (?, ?, ?, ?)")
            .bind(&form.title)
            .bind(&slug)
            .bind(&form.content)
            .bind(cover)
            .execute(&state.pool)
            .await;

    let insert_result = match result {
        Ok(r) => r,
        Err(e) => {
            eprintln!("Failed to insert article: {e}");
            return Html("Database error while creating article").into_response();
        }
    };

    let new_id = insert_result.last_insert_rowid();

    crate::tags::set_article_tags(&state.pool, new_id, &form.tag_ids).await;

    let new_article = sqlx::query_as::<_, Article>(
        "SELECT id, title, slug, content, cover_image, COALESCE(views, 0) as views, COALESCE(created_at, CURRENT_TIMESTAMP) as created_at FROM articles WHERE id = ?"
    )
    .bind(new_id)
    .fetch_optional(&state.pool)
    .await
    .unwrap_or(None);

    match new_article {
        Some(mut p) => {
            p.reading_time = crate::utils::calculate_reading_time(&p.content);
            p.tags = crate::tags::get_tags_for_article(&state.pool, new_id).await;
            ArticleItemTemplate { article: p }.into_response()
        }
        None => {
            Html("Article created, but failed to load it back. Refresh the page.").into_response()
        }
    }
}

/// GET /admin/articles/:id/edit -> Show edit article page.
pub async fn edit_article_page(
    State(state): State<AppState>,
    session: Session,
    Path(id): Path<i64>,
) -> impl IntoResponse {
    let logged_in: Option<bool> = session.get("admin_logged_in").await.unwrap_or(None);
    if logged_in != Some(true) {
        return Redirect::to("/admin/login").into_response();
    }

    let article = sqlx::query_as::<_, Article>(
        "SELECT id, title, slug, content, cover_image, COALESCE(views, 0) as views, COALESCE(created_at, CURRENT_TIMESTAMP) as created_at FROM articles WHERE id = ?"
    )
    .bind(id)
    .fetch_optional(&state.pool)
    .await
    .unwrap_or(None);

    match article {
        Some(p) => {
            let article_tags = crate::tags::get_tags_for_article(&state.pool, id).await;
            let selected_ids: std::collections::HashSet<i64> =
                article_tags.iter().map(|t| t.id).collect();

            let all_tags = crate::tags::get_all_tags(&state.pool).await;
            let tag_checkboxes = all_tags
                .into_iter()
                .map(|t| {
                    let checked = selected_ids.contains(&t.id);
                    (t, checked)
                })
                .collect();

            EditArticleTemplate {
                blog_name: state.config.blog_name,
                active_page: "articles",
                article: p,
                error: None,
                tag_checkboxes,
            }
            .into_response()
        }
        None => Html("Article not found").into_response(),
    }
}

/// POST /admin/articles/:id/edit -> Update article.
pub async fn edit_article(
    State(state): State<AppState>,
    session: Session,
    Path(id): Path<i64>,
    Form(form): Form<CreateArticleForm>,
) -> impl IntoResponse {
    let logged_in: Option<bool> = session.get("admin_logged_in").await.unwrap_or(None);
    if logged_in != Some(true) {
        return Redirect::to("/admin/login").into_response();
    }

    let slug = generate_unique_slug(&state.pool, &form.title, Some(id)).await;

    let cover = match form.cover_image {
        Some(ref url) if url.trim().is_empty() => None,
        other => other,
    };

    let result = sqlx::query(
        "UPDATE articles SET title = ?, slug = ?, content = ?, cover_image = ? WHERE id = ?",
    )
    .bind(&form.title)
    .bind(&slug)
    .bind(&form.content)
    .bind(cover)
    .bind(id)
    .execute(&state.pool)
    .await;

    if result.is_err() {
        return Html("Database error while updating article").into_response();
    }

    crate::tags::set_article_tags(&state.pool, id, &form.tag_ids).await;

    Redirect::to("/admin/articles").into_response()
}

/// DELETE /admin/articles/:id -> Remove article.
pub async fn delete_article(
    State(state): State<AppState>,
    session: Session,
    Path(id): Path<i64>,
) -> impl IntoResponse {
    let logged_in: Option<bool> = session.get("admin_logged_in").await.unwrap_or(None);
    if logged_in != Some(true) {
        return axum::http::StatusCode::UNAUTHORIZED.into_response();
    }

    let _ = sqlx::query("DELETE FROM articles WHERE id = ?")
        .bind(id)
        .execute(&state.pool)
        .await;

    axum::http::StatusCode::OK.into_response()
}

/// GET /admin/tags -> Show tags list.
pub async fn tags_page(State(state): State<AppState>, session: Session) -> impl IntoResponse {
    let logged_in: Option<bool> = session.get("admin_logged_in").await.unwrap_or(None);
    if logged_in != Some(true) {
        return Redirect::to("/admin/login").into_response();
    }

    let tags = crate::tags::get_all_tags(&state.pool).await;

    TagsTemplate {
        blog_name: state.config.blog_name,
        active_page: "tags",
        tags,
    }
    .into_response()
}

/// POST /admin/tags/new -> Create new tag.
pub async fn create_tag(
    State(state): State<AppState>,
    session: Session,
    Form(form): Form<CreateTagForm>,
) -> impl IntoResponse {
    let logged_in: Option<bool> = session.get("admin_logged_in").await.unwrap_or(None);
    if logged_in != Some(true) {
        return Redirect::to("/admin/login").into_response();
    }

    let name = form.name.trim().to_string();
    if name.is_empty() {
        return Html("Tag name cannot be empty").into_response();
    }

    let slug = crate::tags::slugify(&name);

    let result = sqlx::query("INSERT OR IGNORE INTO tags (name, slug) VALUES (?, ?)")
        .bind(&name)
        .bind(&slug)
        .execute(&state.pool)
        .await;

    let insert_result = match result {
        Ok(r) => r,
        Err(e) => {
            eprintln!("Failed to insert tag: {e}");
            return Html("Database error while creating tag").into_response();
        }
    };

    let new_id = insert_result.last_insert_rowid();

    let new_tag = sqlx::query_as::<_, Tag>("SELECT id, name, slug FROM tags WHERE id = ?")
        .bind(new_id)
        .fetch_optional(&state.pool)
        .await
        .unwrap_or(None);

    match new_tag {
        Some(tag) => TagItemTemplate { tag }.into_response(),
        None => Html("Tag created, but failed to load it back.").into_response(),
    }
}

/// DELETE /admin/tags/:id -> Remove tag.
pub async fn delete_tag(
    State(state): State<AppState>,
    session: Session,
    Path(id): Path<i64>,
) -> impl IntoResponse {
    let logged_in: Option<bool> = session.get("admin_logged_in").await.unwrap_or(None);
    if logged_in != Some(true) {
        return axum::http::StatusCode::UNAUTHORIZED.into_response();
    }

    let _ = sqlx::query("DELETE FROM tags WHERE id = ?")
        .bind(id)
        .execute(&state.pool)
        .await;

    axum::http::StatusCode::OK.into_response()
}
