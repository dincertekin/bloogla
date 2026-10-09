//! The dashboard: a "Get your site ready" list for new sites, totals, views
//! over the last 30 days, top posts and referring sites.

use crate::app::models::CurrentUser;
use crate::app::state::AppState;
use crate::db::{or_log, settings};
use crate::services::updates::{self, Release};

use askama::Template;
use axum::extract::{Extension, State};
use axum::response::{IntoResponse, Redirect};
use tower_sessions::Session;

/// Width and height of the views chart, in SVG units.
const CHART_W: f64 = 600.0;
const CHART_H: f64 = 180.0;
const CHART_PAD: f64 = 10.0;

#[derive(Template)]
#[template(path = "dashboard.html")]
pub struct DashboardTemplate {
    pub blog_name: String,
    pub me: CurrentUser,
    /// A newer Bloogla (admins only), and the version running now.
    pub update: Option<Release>,
    pub current_version: &'static str,
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
    /// First steps for a new site (admins only; empty once hidden or done).
    pub checklist: Vec<Step>,
    pub steps_done: usize,
}

/// One item of "Get your site ready".
pub struct Step {
    pub text: &'static str,
    /// Why it matters, in one short sentence.
    pub hint: &'static str,
    /// Where to do it.
    pub url: &'static str,
    pub done: bool,
}

/// The first steps, and whether each is done.
async fn checklist(state: &AppState, me: &CurrentUser) -> Vec<Step> {
    let site = settings::load(&state.pool).await;
    if !me.is_admin() || site.checklist_hidden {
        return Vec::new();
    }
    // Sample posts are added at setup, so a post written more than a few
    // minutes after the first account was made is the owner's own.
    let wrote_a_post: bool = or_log(
        sqlx::query_scalar(
            "SELECT EXISTS (SELECT 1 FROM posts WHERE is_page = 0
             AND created_at > (SELECT datetime(MIN(created_at), '+5 minutes') FROM users))",
        )
        .fetch_one(&state.pool)
        .await,
        "checklist posts",
    );
    let two_factor = crate::db::users::two_factor(&state.pool, me.id)
        .await
        .is_some();
    let steps = vec![
        Step {
            text: me.t("Describe your site"),
            hint: me.t("Search engines and link previews show it."),
            url: "/admin/settings#site",
            done: !site.blog_description.trim().is_empty(),
        },
        Step {
            text: me.t("Add a site icon"),
            hint: me.t("The small picture in browser tabs and bookmarks."),
            url: "/admin/settings#site",
            done: !site.site_icon.is_empty(),
        },
        Step {
            text: me.t("Write your first post"),
            hint: me.t("Or edit the sample content to make it yours."),
            url: "/admin/posts/new",
            done: wrote_a_post,
        },
        Step {
            text: me.t("Set up email"),
            hint: me.t("So you hear about new comments and can reset a forgotten password."),
            url: "/admin/settings#email",
            done: !site.smtp_host.is_empty(),
        },
        Step {
            text: me.t("Turn on two-factor login"),
            hint: me.t("Keeps your site safe even if your password leaks."),
            url: "/admin/profile#two-factor",
            done: two_factor,
        },
    ];
    if steps.iter().all(|s| s.done) {
        Vec::new()
    } else {
        steps
    }
}

/// POST /admin/checklist/hide -> Stop showing "Get your site ready".
pub async fn hide_checklist(State(state): State<AppState>) -> Redirect {
    if let Err(e) = settings::save(&state.pool, &[("checklist_hidden", "true".into())]).await {
        tracing::error!("Failed to hide the checklist: {e}");
    }
    Redirect::to("/admin")
}

/// A post in the "most viewed" chart.
#[derive(sqlx::FromRow)]
pub struct TopPost {
    pub id: i64,
    pub title: String,
    pub views: i64,
}

/// Session key set when setup finishes: the Dashboard shows "Your site is
/// ready" once, then removes it.
pub const SESSION_WELCOME: &str = "welcome";

/// GET /admin -> Show admin panel/dashboard.
pub async fn page(
    State(state): State<AppState>,
    Extension(me): Extension<CurrentUser>,
    session: Session,
) -> impl IntoResponse {
    let welcome = session
        .remove::<bool>(SESSION_WELCOME)
        .await
        .ok()
        .flatten()
        .unwrap_or(false);
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

    let update = if me.is_admin() {
        updates::available(&state.newer_release)
    } else {
        None
    };

    let blog_name = settings::load(&state.pool).await.blog_name.clone();
    let checklist = checklist(&state, &me).await;
    let steps_done = checklist.iter().filter(|s| s.done).count();

    DashboardTemplate {
        update,
        current_version: updates::CURRENT_VERSION,
        blog_name,
        me,
        welcome,
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
        checklist,
        steps_done,
    }
}
