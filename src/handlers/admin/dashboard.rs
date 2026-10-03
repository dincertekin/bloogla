//! The dashboard: totals, views over the last 30 days, top posts and
//! referring sites.

use crate::app::models::CurrentUser;
use crate::app::state::AppState;
use crate::db::{or_log, settings};

use askama::Template;
use axum::extract::{Extension, Query, State};
use axum::response::IntoResponse;

/// Width and height of the views chart, in SVG units.
const CHART_W: f64 = 600.0;
const CHART_H: f64 = 180.0;
const CHART_PAD: f64 = 10.0;

#[derive(Template)]
#[template(path = "dashboard.html")]
pub struct DashboardTemplate {
    pub blog_name: String,
    pub me: CurrentUser,
    pub pending_comments: i64,
    pub welcome: bool,
    pub total_posts: i64,
    pub total_drafts: i64,
    pub total_views: i64,
    pub top_posts: Vec<TopPost>,
    pub max_views: i64,
    /// Views in the last 30 days, and the chart line for them.
    pub views_30d: i64,
    pub views_points: String,
    pub chart_start: String,
    /// Sites that sent visitors in the last 30 days.
    pub referrers: Vec<(String, i64)>,
    pub max_referrer: i64,
}

/// A post in the "most viewed" chart.
#[derive(sqlx::FromRow)]
pub struct TopPost {
    pub id: i64,
    pub title: String,
    pub views: i64,
}

#[derive(serde::Deserialize)]
pub struct DashboardQuery {
    welcome: Option<String>,
}

/// GET /admin -> Show admin panel/dashboard.
pub async fn page(
    State(state): State<AppState>,
    Extension(me): Extension<CurrentUser>,
    Query(query): Query<DashboardQuery>,
) -> impl IntoResponse {
    let (total_posts, total_drafts, total_views): (i64, i64, i64) = or_log(
        sqlx::query_as(
            "SELECT COUNT(*) FILTER (WHERE status != 'draft'),
                    COUNT(*) FILTER (WHERE status = 'draft'),
                    COALESCE(SUM(views), 0)
             FROM posts WHERE is_page = 0",
        )
        .fetch_one(&state.pool)
        .await,
        "dashboard totals",
    );
    let top_posts: Vec<TopPost> = or_log(
        sqlx::query_as(
            "SELECT id, title, views FROM posts
             WHERE is_page = 0 AND views > 0 ORDER BY views DESC LIMIT 5",
        )
        .fetch_all(&state.pool)
        .await,
        "top posts",
    );
    let max_views = top_posts.first().map_or(1, |p| p.views.max(1));

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

    let views_points = series
        .iter()
        .enumerate()
        .map(|(i, (_, value))| {
            let x = i as f64 / (series.len() - 1) as f64 * CHART_W;
            let y = CHART_H
                - CHART_PAD
                - (*value as f64 / views_peak as f64) * (CHART_H - 2.0 * CHART_PAD);
            format!("{x:.1},{y:.1}")
        })
        .collect::<Vec<_>>()
        .join(" ");
    let chart_start = me.lang.day_month(series[0].0);

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

    let pending_comments: i64 = if me.can_edit_all() {
        or_log(
            sqlx::query_scalar("SELECT COUNT(*) FROM comments WHERE status = 'pending'")
                .fetch_one(&state.pool)
                .await,
            "count pending comments",
        )
    } else {
        0
    };

    let blog_name = settings::load(&state.pool).await.blog_name.clone();

    DashboardTemplate {
        pending_comments,
        blog_name,
        me,
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
