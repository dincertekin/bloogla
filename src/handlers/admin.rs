use crate::models::{
    AppState, CreateTagForm, GeneralSettingsForm, Post, Tag, UpdatePasswordForm, UpdateThemeForm,
    POST_SELECT,
};
use crate::templates::{AdminTemplate, SettingsTemplate, TagItemTemplate, TagRow, TagsTemplate};
use crate::themes::discover_themes;
use crate::utils::{get_setting, or_log};

use argon2::{
    password_hash::{PasswordHash, PasswordHasher, PasswordVerifier, SaltString},
    Argon2,
};
use axum::extract::{Path, Query, State};
use axum::http::StatusCode;
use axum::response::{Html, IntoResponse, Redirect, Response};
use axum_extra::extract::Form;
use rand::thread_rng;

#[derive(serde::Deserialize)]
pub struct DashboardQuery {
    welcome: Option<String>,
}

/// GET /admin -> Show admin panel/dashboard.
pub async fn admin_dashboard(
    State(state): State<AppState>,
    Query(query): Query<DashboardQuery>,
) -> impl IntoResponse {
    let posts = or_log(
        sqlx::query_as::<_, Post>(&format!("{POST_SELECT} WHERE is_page = 0 ORDER BY id DESC"))
            .fetch_all(&state.pool)
            .await,
        "dashboard posts",
    );

    let total_posts = posts.iter().filter(|p| p.status != "draft").count() as i64;
    let total_drafts = posts.iter().filter(|p| p.status == "draft").count() as i64;
    let total_views: i64 = posts.iter().map(|p| p.views).sum();

    let mut top_posts = posts.clone();
    top_posts.retain(|p| p.views > 0);
    top_posts.sort_by_key(|p| std::cmp::Reverse(p.views));
    top_posts.truncate(5);

    let max_views = top_posts.first().map(|p| p.views).unwrap_or(0).max(1);

    // Views per day for the last 30 days, with missing days as zero.
    let daily: std::collections::HashMap<String, i64> = or_log(
        sqlx::query_as(
            "SELECT day, SUM(views) FROM daily_views
             WHERE day >= date('now', '-29 days') GROUP BY day",
        )
        .fetch_all(&state.pool)
        .await,
        "daily views",
    )
    .into_iter()
    .collect();
    let today = chrono::Utc::now().date_naive();
    let series: Vec<(chrono::NaiveDate, i64)> = (0..30)
        .rev()
        .map(|ago| {
            let day = today - chrono::Duration::days(ago);
            let views = daily.get(&day.to_string()).copied().unwrap_or(0);
            (day, views)
        })
        .collect();
    let views_30d: i64 = series.iter().map(|(_, v)| v).sum();
    let views_peak = series.iter().map(|(_, v)| *v).max().unwrap_or(0).max(1);

    const CHART_W: f64 = 600.0;
    const CHART_H: f64 = 180.0;
    const PAD: f64 = 10.0;
    let views_points = series
        .iter()
        .enumerate()
        .map(|(i, (_, value))| {
            let x = i as f64 / (series.len() - 1) as f64 * CHART_W;
            let y = CHART_H - PAD - (*value as f64 / views_peak as f64) * (CHART_H - 2.0 * PAD);
            format!("{x:.1},{y:.1}")
        })
        .collect::<Vec<_>>()
        .join(" ");
    let chart_start = series[0].0.format("%b %-d").to_string();

    let referrers: Vec<(String, i64)> = or_log(
        sqlx::query_as(
            "SELECT host, SUM(views) AS total FROM daily_referrers
             WHERE day >= date('now', '-29 days')
             GROUP BY host ORDER BY total DESC LIMIT 5",
        )
        .fetch_all(&state.pool)
        .await,
        "top referrers",
    );
    let max_referrer = referrers.first().map(|(_, v)| *v).unwrap_or(1).max(1);

    let blog_name = get_setting(&state.pool, "blog_name", "Bloogla").await;

    AdminTemplate {
        blog_name,
        welcome: query.welcome.is_some(),
        total_posts,
        total_drafts,
        total_views,
        top_posts,
        max_views,
        views_30d,
        views_points,
        chart_start,
        referrers,
        max_referrer,
    }
}

/// GET /admin/tags -> Show tags list.
pub async fn tags_page(State(state): State<AppState>) -> impl IntoResponse {
    let rows: Vec<(i64, String, String, i64)> = or_log(
        sqlx::query_as(
            "SELECT t.id, t.name, t.slug, COUNT(pt.post_id)
             FROM tags t LEFT JOIN post_tags pt ON pt.tag_id = t.id
             GROUP BY t.id ORDER BY t.name ASC",
        )
        .fetch_all(&state.pool)
        .await,
        "list tags",
    );
    let tags = rows
        .into_iter()
        .map(|(id, name, slug, post_count)| TagRow {
            tag: Tag { id, name, slug },
            post_count,
        })
        .collect();
    let blog_name = get_setting(&state.pool, "blog_name", "Bloogla").await;

    TagsTemplate { blog_name, tags }
}

/// Show a message in the tag form instead of adding a row to the list.
fn tag_form_error(message: &str) -> Response {
    (
        [
            ("HX-Retarget", "#tag-form-alert"),
            ("HX-Reswap", "innerHTML"),
        ],
        Html(format!(r#"<div class="alert alert-error">{message}</div>"#)),
    )
        .into_response()
}

/// POST /admin/tags/new -> Create new tag.
pub async fn create_tag(
    State(state): State<AppState>,
    Form(form): Form<CreateTagForm>,
) -> Response {
    let name = form.name.trim().to_string();
    if name.is_empty() {
        return tag_form_error("Enter a name for the tag.");
    }

    let slug = crate::tags::slugify(&name);

    let result = sqlx::query("INSERT OR IGNORE INTO tags (name, slug) VALUES (?, ?)")
        .bind(&name)
        .bind(&slug)
        .execute(&state.pool)
        .await;

    let new_id = match result {
        Ok(r) if r.rows_affected() == 0 => {
            return tag_form_error("A tag with this name already exists.");
        }
        Ok(r) => r.last_insert_rowid(),
        Err(e) => {
            eprintln!("Failed to insert tag: {e}");
            return tag_form_error("Could not save the tag. Please try again.");
        }
    };

    TagItemTemplate {
        row: TagRow {
            tag: Tag {
                id: new_id,
                name,
                slug,
            },
            post_count: 0,
        },
    }
    .into_response()
}

/// DELETE /admin/tags/:id -> Remove tag.
pub async fn delete_tag(State(state): State<AppState>, Path(id): Path<i64>) -> impl IntoResponse {
    let result = sqlx::query("DELETE FROM tags WHERE id = ?")
        .bind(id)
        .execute(&state.pool)
        .await;

    match result {
        Ok(r) if r.rows_affected() > 0 => StatusCode::OK,
        Ok(_) => StatusCode::NOT_FOUND,
        Err(_) => StatusCode::INTERNAL_SERVER_ERROR,
    }
}

/// GET /admin/settings -> Render settings page.
pub async fn settings_page(State(state): State<AppState>) -> impl IntoResponse {
    let blog_name = get_setting(&state.pool, "blog_name", "Bloogla").await;
    let blog_description = get_setting(&state.pool, "blog_description", "").await;
    let blog_keywords = get_setting(&state.pool, "blog_keywords", "").await;
    let active_theme = get_setting(&state.pool, "active_theme", "default").await;
    let posts_per_page = get_setting(&state.pool, "posts_per_page", "10").await;
    let nav_menu = get_setting(&state.pool, "nav_menu", "").await;
    let show_views = get_setting(&state.pool, "show_views", "false").await == "true";
    let publisher_name = get_setting(&state.pool, "publisher_name", "").await;
    let publisher_name = if publisher_name.contains('@') {
        String::new()
    } else {
        publisher_name
    };
    let publisher_type = get_setting(&state.pool, "publisher_type", "Person").await;
    let site_icon = get_setting(&state.pool, "site_icon", "").await;

    let available_themes = discover_themes();

    SettingsTemplate {
        blog_name,
        blog_description,
        blog_keywords,
        posts_per_page,
        nav_menu,
        show_views,
        publisher_name,
        publisher_type,
        site_icon,
        active_theme,
        available_themes,
    }
}

/// POST /admin/settings/general -> Update general blog settings via HTMX.
pub async fn update_general_settings(
    State(state): State<AppState>,
    Form(form): Form<GeneralSettingsForm>,
) -> impl IntoResponse {
    let mut settings: Vec<(&str, String)> = Vec::new();

    // "Site" card
    if let Some(blog_name) = form.blog_name {
        let blog_name = blog_name.trim().to_string();
        if blog_name.is_empty() {
            return settings_error("Give your site a title.");
        }
        let publisher_type = match form.publisher_type.as_deref() {
            Some("Organization") => "Organization",
            _ => "Person",
        };
        let site_icon = form.site_icon.unwrap_or_default().trim().to_string();
        if !(site_icon.is_empty()
            || site_icon.starts_with("/uploads/")
            || site_icon.starts_with("https://"))
        {
            return settings_error("Choose the site icon from your media library.");
        }
        settings.extend([
            ("blog_name", blog_name),
            (
                "blog_description",
                form.blog_description.unwrap_or_default().trim().to_string(),
            ),
            (
                "blog_keywords",
                form.blog_keywords.unwrap_or_default().trim().to_string(),
            ),
            (
                "publisher_name",
                form.publisher_name.unwrap_or_default().trim().to_string(),
            ),
            ("publisher_type", publisher_type.to_string()),
            ("site_icon", site_icon),
        ]);
    }

    // "Reading" card (always sends posts_per_page)
    if let Some(per_page) = form.posts_per_page {
        let posts_per_page = match per_page.trim() {
            "" => "10".to_string(),
            v => match v.parse::<u32>() {
                Ok(n @ 1..=100) => n.to_string(),
                _ => return settings_error("Posts per page must be a number from 1 to 100."),
            },
        };
        let nav_menu = form.nav_menu.unwrap_or_default().trim().to_string();
        let menu_lines = nav_menu.lines().filter(|l| !l.trim().is_empty()).count();
        if crate::utils::parse_menu(&nav_menu).len() != menu_lines {
            return settings_error(
                "Each menu line must look like <code>Label | /path</code> or \
                 <code>Label | https://example.com</code>.",
            );
        }
        settings.extend([
            ("posts_per_page", posts_per_page),
            ("nav_menu", nav_menu),
            ("show_views", form.show_views.is_some().to_string()),
        ]);
    }

    for (key, value) in settings {
        if let Err(e) = crate::utils::set_setting(&state.pool, key, &value).await {
            eprintln!("Failed to save setting {key}: {e}");
            return settings_error("Couldn't save your changes. Please try again.");
        }
    }

    Html(r#"<div class="alert alert-success">Saved.</div>"#).into_response()
}

fn settings_error(message: &str) -> Response {
    Html(format!(r#"<div class="alert alert-error">{message}</div>"#)).into_response()
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
        return Html(r#"<div class="alert alert-error">The new passwords don't match.</div>"#)
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

    Html(r#"<div class="alert alert-success">Password updated.</div>"#).into_response()
}

/// POST /admin/settings/theme -> Update theme setting.
pub async fn update_theme(
    State(state): State<AppState>,
    Form(form): Form<UpdateThemeForm>,
) -> impl IntoResponse {
    if !discover_themes().iter().any(|t| t.id == form.theme_name) {
        return (StatusCode::BAD_REQUEST, "Unknown theme").into_response();
    }

    if let Err(e) = crate::utils::set_setting(&state.pool, "active_theme", &form.theme_name).await {
        eprintln!("Failed to save theme: {e}");
    }

    Redirect::to("/admin/settings").into_response()
}
