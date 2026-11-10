//! The dashboard: a "Get your site ready" list for new sites, then boxes each
//! person can reorder or hide (Customize): site health (HTTPS, backups,
//! email, updates), totals, views over the last 30 days, referring sites and
//! top posts.

use crate::app::models::CurrentUser;
use crate::app::state::AppState;
use crate::content::text::display_datetime;
use crate::db::analytics::{self, TopPost};
use crate::db::settings;
use crate::services::backup as backups;
use crate::services::updates;

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
    /// The boxes below the checklist, in this person's order.
    pub sections: Vec<Section>,
    /// Whether the site is safe and looked after (admins only).
    pub health: Vec<HealthRow>,
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

/// A box on the Dashboard that each person can move or hide (Customize).
pub struct Section {
    /// `health`, `stats`, `views`, `referrers` or `top_posts`.
    pub id: &'static str,
    /// Its name in the Customize panel.
    pub name: &'static str,
    pub shown: bool,
}

/// Every box, in the usual order. Site health is for admins only.
const SECTIONS: [&str; 5] = ["health", "stats", "views", "referrers", "top_posts"];

/// The boxes `me` may see, in the order they saved (`stats,-views,...`, a
/// `-` for hidden ones). Boxes missing from what they saved, like ones added
/// in a later version, come at the end, shown.
fn sections(me: &CurrentUser, saved: &str) -> Vec<Section> {
    let allowed = |id: &str| id != "health" || me.is_admin();
    let mut ids: Vec<(&'static str, bool)> = Vec::new();
    for part in saved.split(',') {
        let (id, shown) = match part.trim().strip_prefix('-') {
            Some(hidden) => (hidden, false),
            None => (part.trim(), true),
        };
        let known = SECTIONS.iter().find(|known| **known == id);
        if let Some(id) = known {
            if !ids.iter().any(|(seen, _)| seen == id) {
                ids.push((id, shown));
            }
        }
    }
    for id in SECTIONS {
        if !ids.iter().any(|(seen, _)| *seen == id) {
            ids.push((id, true));
        }
    }
    ids.into_iter()
        .filter(|(id, _)| allowed(id))
        .map(|(id, shown)| Section {
            id,
            name: match id {
                "health" => me.t("Site health"),
                "stats" => me.t("Totals"),
                "views" => me.t("Last 30 days"),
                "referrers" => me.t("Where readers come from"),
                _ => me.t("Most read"),
            },
            shown,
        })
        .collect()
}

#[derive(serde::Deserialize)]
pub struct LayoutForm {
    layout: String,
}

/// POST /admin/dashboard/layout -> Remember the order of this person's
/// Dashboard boxes and which are hidden (dashboard.js sends it after each
/// change). Only known boxes are kept.
pub async fn save_layout(
    State(state): State<AppState>,
    Extension(me): Extension<CurrentUser>,
    axum_extra::extract::Form(form): axum_extra::extract::Form<LayoutForm>,
) -> axum::http::StatusCode {
    let layout = sections(&me, &form.layout)
        .iter()
        .map(|section| {
            if section.shown {
                section.id.to_string()
            } else {
                format!("-{}", section.id)
            }
        })
        .collect::<Vec<_>>()
        .join(",");
    match crate::db::users::set_dashboard_layout(&state.pool, me.id, &layout).await {
        Ok(()) => axum::http::StatusCode::NO_CONTENT,
        Err(e) => {
            tracing::error!("Failed to save the Dashboard layout: {e}");
            axum::http::StatusCode::INTERNAL_SERVER_ERROR
        }
    }
}

/// One line of "Site health": something that's fine, or something to fix.
pub struct HealthRow {
    pub ok: bool,
    pub text: String,
    /// Where to fix or read more: `(address, link text)`.
    pub link: Option<(String, &'static str)>,
}

/// Days without a daily backup before Site health says something's wrong.
const BACKUP_LATE_DAYS: i64 = 2;

/// "Site health": HTTPS, backups (and their copy in storage), email and updates, for admins.
async fn site_health(state: &AppState, me: &CurrentUser) -> Vec<HealthRow> {
    if !me.is_admin() {
        return Vec::new();
    }
    let site = settings::load(&state.pool).await;
    let mut rows = Vec::new();

    // A site tried out on someone's own computer doesn't need HTTPS.
    let base_url = &state.config.base_url;
    let on_this_computer = ["http://localhost", "http://127.0.0.1"]
        .iter()
        .any(|local| base_url.starts_with(local));
    if !on_this_computer {
        let https = base_url.starts_with("https://");
        rows.push(HealthRow {
            ok: https,
            text: if https {
                me.t("Visitors reach your site over HTTPS.").to_string()
            } else {
                me.t("Your site doesn't use HTTPS, so browsers mark it as not secure.")
                    .to_string()
            },
            link: (!https).then(|| {
                (
                    "https://github.com/dincertekin/bloogla/blob/main/docs/running.md#on-a-server"
                        .to_string(),
                    me.t("How to turn it on"),
                )
            }),
        });
    }

    let backup_settings = Some(("/admin/settings#backup".to_string(), me.t("Backup")));
    let last_backup = backups::last_automatic_backup().map(chrono::DateTime::<chrono::Utc>::from);
    rows.push(match (site.auto_backup, last_backup) {
        (false, _) => HealthRow {
            ok: false,
            text: me.t("Daily backups are off.").to_string(),
            link: backup_settings,
        },
        (true, None) => HealthRow {
            ok: true,
            text: me.t("Daily backups are on. The first one is saved within the hour.").to_string(),
            link: None,
        },
        (true, Some(saved)) => {
            let late = chrono::Utc::now() - saved > chrono::Duration::days(BACKUP_LATE_DAYS);
            let when = display_datetime(
                me.lang,
                site.timezone,
                &saved.format("%Y-%m-%d %H:%M:%S").to_string(),
            );
            HealthRow {
                ok: !late,
                text: if late {
                    me.tv("The last daily backup is from {date}. Check that the server has free space.", when)
                } else {
                    me.tv("Daily backups are on. Last one: {date}.", when)
                },
                link: late.then_some(backup_settings).flatten(),
            }
        }
    });

    if site.storage_on {
        rows.push(if !site.storage_last_problem.is_empty() {
            HealthRow {
                ok: false,
                text: format!(
                    "{} {}",
                    me.t("The last copy to your storage failed."),
                    me.lang.t_owned(&site.storage_last_problem)
                ),
                link: Some(("/admin/settings#backup".to_string(), me.t("Backup"))),
            }
        } else if site.storage_last_copied.is_empty() {
            HealthRow {
                ok: true,
                text: me
                    .t("Backups will be copied to your storage within the hour.")
                    .to_string(),
                link: None,
            }
        } else {
            let when = display_datetime(me.lang, site.timezone, &site.storage_last_copied);
            HealthRow {
                ok: true,
                text: me.tv(
                    "Backups are copied to your storage. Last copy: {date}.",
                    when,
                ),
                link: None,
            }
        });
    }

    let email = !site.smtp_host.is_empty();
    rows.push(HealthRow {
        ok: email,
        text: if email {
            me.t("Email is set up.").to_string()
        } else {
            me.t("Email isn't set up, so you won't hear about new comments and can't reset a forgotten password.")
                .to_string()
        },
        link: (!email).then(|| ("/admin/settings#email".to_string(), me.t("Set up email"))),
    });

    let current = updates::CURRENT_VERSION;
    rows.push(match updates::available(&state.newer_release) {
        Some(release) => HealthRow {
            ok: false,
            text: me.tv2(
                "Bloogla {version} is available. You have {current}.",
                &release.version,
                current,
            ),
            link: Some(("/admin/settings#updates".to_string(), me.t("Updates"))),
        },
        None if site.update_check_daily => HealthRow {
            ok: true,
            text: me.tv("Bloogla {version} is up to date.", current),
            link: None,
        },
        // Checking stays off until the owner turns it on, so this isn't a problem.
        None => HealthRow {
            ok: true,
            text: me.tv(
                "You have Bloogla {version}. Checking for new versions is off.",
                current,
            ),
            link: Some(("/admin/settings#updates".to_string(), me.t("Updates"))),
        },
    });

    rows
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
    let wrote_a_post = crate::db::posts::owner_wrote_a_post(&state.pool).await;
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
    let (total_posts, total_drafts, total_views) = analytics::totals(&state.pool).await;
    let top_posts = analytics::top_posts(&state.pool).await;
    let max_views = top_posts.first().map_or(1, |p| p.views.max(1));

    // Views per day for the last 30 days, with missing days as zero.
    let daily = analytics::daily_views(&state.pool).await;
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

    let referrers = analytics::top_referrers(&state.pool).await;
    let max_referrer = referrers.first().map(|(_, v)| *v).unwrap_or(1).max(1);

    let health = site_health(&state, &me).await;
    let saved_layout = crate::db::users::dashboard_layout(&state.pool, me.id).await;
    let sections = sections(&me, &saved_layout);

    let blog_name = settings::load(&state.pool).await.blog_name.clone();
    let checklist = checklist(&state, &me).await;
    let steps_done = checklist.iter().filter(|s| s.done).count();

    DashboardTemplate {
        health,
        sections,
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
