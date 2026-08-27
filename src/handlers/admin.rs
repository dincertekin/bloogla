use crate::models::{
    AppState, Article, CreateArticleForm, CreateTagForm, GeneralSettingsForm, Tag,
    UpdatePasswordForm, UpdateThemeForm,
};
use crate::templates::{
    AdminTemplate, ArticleItemTemplate, ArticlesTemplate, EditArticleTemplate, SettingsTemplate,
    TagItemTemplate, TagsTemplate,
};
use crate::themes::discover_themes;
use crate::utils::{generate_unique_slug, get_setting};

use argon2::{
    password_hash::{PasswordHash, PasswordHasher, PasswordVerifier, SaltString},
    Argon2,
};
use axum::extract::{Path, State};
use axum::response::{Html, IntoResponse, Redirect};
use axum_extra::extract::Form;
use rand::thread_rng;

/// GET /admin -> Show admin panel/dashboard.
pub async fn admin_dashboard(State(state): State<AppState>) -> impl IntoResponse {
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

    let blog_name = get_setting(&state.pool, "blog_name", "Bloogla").await;

    AdminTemplate {
        blog_name,
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
}

/// GET /admin/articles -> List all articles.
pub async fn list_articles(State(state): State<AppState>) -> impl IntoResponse {
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
    let blog_name = get_setting(&state.pool, "blog_name", "Bloogla").await;

    ArticlesTemplate {
        blog_name,
        articles,
        all_tags,
    }
}

/// POST /admin/articles/new -> Create new article.
pub async fn create_article(
    State(state): State<AppState>,
    Form(form): Form<CreateArticleForm>,
) -> impl IntoResponse {
    let slug = generate_unique_slug(&state.pool, &form.title, None)
        .await
        .unwrap_or_default();

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
    Path(id): Path<i64>,
) -> impl IntoResponse {
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

            let blog_name = get_setting(&state.pool, "blog_name", "Bloogla").await;

            EditArticleTemplate {
                blog_name,
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
    Path(id): Path<i64>,
    Form(form): Form<CreateArticleForm>,
) -> impl IntoResponse {
    let slug = generate_unique_slug(&state.pool, &form.title, Some(id))
        .await
        .unwrap_or_default();

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
    Path(id): Path<i64>,
) -> impl IntoResponse {
    let _ = sqlx::query("DELETE FROM articles WHERE id = ?")
        .bind(id)
        .execute(&state.pool)
        .await;

    axum::http::StatusCode::OK
}

/// GET /admin/tags -> Show tags list.
pub async fn tags_page(State(state): State<AppState>) -> impl IntoResponse {
    let tags = crate::tags::get_all_tags(&state.pool).await;
    let blog_name = get_setting(&state.pool, "blog_name", "Bloogla").await;

    TagsTemplate { blog_name, tags }
}

/// POST /admin/tags/new -> Create new tag.
pub async fn create_tag(
    State(state): State<AppState>,
    Form(form): Form<CreateTagForm>,
) -> impl IntoResponse {
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
pub async fn delete_tag(State(state): State<AppState>, Path(id): Path<i64>) -> impl IntoResponse {
    let _ = sqlx::query("DELETE FROM tags WHERE id = ?")
        .bind(id)
        .execute(&state.pool)
        .await;

    axum::http::StatusCode::OK
}

/// GET /admin/settings -> Render settings page.
pub async fn settings_page(State(state): State<AppState>) -> impl IntoResponse {
    let blog_name = get_setting(&state.pool, "blog_name", "Bloogla").await;
    let blog_description = get_setting(&state.pool, "blog_description", "").await;
    let blog_keywords = get_setting(&state.pool, "blog_keywords", "").await;
    let active_theme = get_setting(&state.pool, "active_theme", "default").await;

    let available_themes = discover_themes();

    SettingsTemplate {
        blog_name,
        blog_description,
        blog_keywords,
        active_theme,
        available_themes,
    }
}

/// POST /admin/settings/general -> Update general blog settings via HTMX.
pub async fn update_general_settings(
    State(state): State<AppState>,
    Form(form): Form<GeneralSettingsForm>,
) -> impl IntoResponse {
    let settings = [
        ("blog_name", form.blog_name.trim()),
        ("blog_description", form.blog_description.trim()),
        ("blog_keywords", form.blog_keywords.trim()),
    ];

    for (key, value) in settings {
        let _ = sqlx::query(
            "INSERT INTO settings (key, value) VALUES (?, ?)
             ON CONFLICT(key) DO UPDATE SET value = excluded.value",
        )
        .bind(key)
        .bind(value)
        .execute(&state.pool)
        .await;
    }

    Html(r#"<div class="alert alert-success">Settings updated.</div>"#)
}

/// POST /admin/settings/password -> Change admin password via HTMX.
pub async fn update_password(
    State(state): State<AppState>,
    Form(form): Form<UpdatePasswordForm>,
) -> impl IntoResponse {
    if form.new_password.len() < 8 {
        return Html(
            r#"<div class="alert alert-error">New password must be at least 8 characters long.</div>"#,
        )
        .into_response();
    }

    if form.new_password != form.confirm_password {
        return Html(
            r#"<div class="alert alert-error">New password and confirmation do not match.</div>"#,
        )
        .into_response();
    }

    let stored_hash: Option<String> =
        sqlx::query_scalar("SELECT password_hash FROM users ORDER BY id ASC LIMIT 1")
            .fetch_optional(&state.pool)
            .await
            .unwrap_or_default();

    let stored_hash = match stored_hash {
        Some(h) => h,
        None => {
            return Html(r#"<div class="alert alert-error">Admin user not found.</div>"#)
                .into_response();
        }
    };

    let parsed_hash = match PasswordHash::new(&stored_hash) {
        Ok(hash) => hash,
        Err(_) => {
            return Html(r#"<div class="alert alert-error">Invalid stored password format.</div>"#)
                .into_response();
        }
    };

    if Argon2::default()
        .verify_password(form.current_password.as_bytes(), &parsed_hash)
        .is_err()
    {
        return Html(r#"<div class="alert alert-error">Current password is incorrect.</div>"#)
            .into_response();
    }

    let salt = SaltString::generate(&mut thread_rng());
    let new_password_hash =
        match Argon2::default().hash_password(form.new_password.as_bytes(), &salt) {
            Ok(h) => h.to_string(),
            Err(e) => {
                return Html(format!(
                    r#"<div class="alert alert-error">Password hashing failed: {}</div>"#,
                    e
                ))
                .into_response();
            }
        };

    let res = sqlx::query("UPDATE users SET password_hash = ? WHERE id = (SELECT id FROM users ORDER BY id ASC LIMIT 1)")
        .bind(&new_password_hash)
        .execute(&state.pool)
        .await;

    if res.is_err() {
        return Html(
            r#"<div class="alert alert-error">Failed to update password in database.</div>"#,
        )
        .into_response();
    }

    Html(r#"<div class="alert alert-success">Password updated successfully!</div>"#).into_response()
}

/// POST /admin/settings/theme -> Update theme setting.
pub async fn update_theme(
    State(state): State<AppState>,
    Form(form): Form<UpdateThemeForm>,
) -> impl IntoResponse {
    let _ = sqlx::query(
        "INSERT INTO settings (key, value) VALUES ('active_theme', ?)
         ON CONFLICT(key) DO UPDATE SET value = excluded.value",
    )
    .bind(&form.theme_name)
    .execute(&state.pool)
    .await;

    Redirect::to("/admin/settings")
}
