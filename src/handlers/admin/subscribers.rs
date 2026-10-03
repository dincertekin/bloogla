//! The newsletter's subscriber list, with CSV export.

use super::StatusFilter;
use crate::app::models::CurrentUser;
use crate::app::state::AppState;
use crate::content::text::display_date;
use crate::db::{or_log, settings};
use crate::services::email::smtp_settings;

use askama::Template;
use axum::extract::{Extension, Path, Query, State};
use axum::http::{header, StatusCode};
use axum::response::{IntoResponse, Response};
use serde::Deserialize;
use std::collections::HashMap;

/// A newsletter subscriber.
pub struct SubscriberRow {
    pub id: i64,
    pub email: String,
    pub date: String,
}

/// Newsletter subscribers page.
#[derive(Template)]
#[template(path = "subscribers.html")]
pub struct SubscribersTemplate {
    pub blog_name: String,
    pub me: CurrentUser,
    pub subscribers: Vec<SubscriberRow>,
    pub filters: Vec<StatusFilter>,
    pub active_filter: String,
    pub newsletter_on: bool,
    pub email_ready: bool,
}

#[derive(Deserialize)]
pub struct ListQuery {
    status: Option<String>,
}

/// GET /admin/subscribers -> Newsletter subscribers.
pub async fn page(
    State(state): State<AppState>,
    Extension(me): Extension<CurrentUser>,
    Query(query): Query<ListQuery>,
) -> Response {
    let active = query
        .status
        .as_deref()
        .filter(|s| matches!(*s, "pending" | "unsubscribed"))
        .unwrap_or("active");
    let counts: HashMap<String, i64> = or_log(
        sqlx::query_as("SELECT status, COUNT(*) FROM subscribers GROUP BY status")
            .fetch_all(&state.pool)
            .await,
        "count subscribers",
    )
    .into_iter()
    .collect();
    let count = |s: &str| counts.get(s).copied().unwrap_or(0) as usize;

    let rows: Vec<(i64, String, String)> = or_log(
        sqlx::query_as(
            "SELECT id, email, COALESCE(confirmed_at, created_at) FROM subscribers
             WHERE status = ? ORDER BY id DESC LIMIT 1000",
        )
        .bind(active)
        .fetch_all(&state.pool)
        .await,
        "list subscribers",
    );

    let lang = me.lang;
    let site = settings::load(&state.pool).await;
    SubscribersTemplate {
        blog_name: site.blog_name.clone(),
        me,
        subscribers: rows
            .into_iter()
            .map(|(id, email, date)| SubscriberRow {
                id,
                email,
                date: display_date(lang, &date),
            })
            .collect(),
        filters: vec![
            StatusFilter::new("active", "Subscribed", count("active")),
            StatusFilter::new("pending", "Not confirmed", count("pending")),
            StatusFilter::new("unsubscribed", "Unsubscribed", count("unsubscribed")),
        ],
        active_filter: active.to_string(),
        newsletter_on: site.newsletter,
        email_ready: smtp_settings(&state).await.is_some(),
    }
    .into_response()
}

/// GET /admin/subscribers.csv -> Active subscribers, for moving to another service.
pub async fn export_csv(State(state): State<AppState>) -> Response {
    let rows: Vec<(String, String)> = or_log(
        sqlx::query_as(
            "SELECT email, COALESCE(confirmed_at, created_at) FROM subscribers
             WHERE status = 'active' ORDER BY id",
        )
        .fetch_all(&state.pool)
        .await,
        "export subscribers",
    );
    let mut csv = String::from("email,subscribed_at\n");
    for (email, date) in rows {
        csv.push_str(&format!("{},{}\n", csv_cell(&email), csv_cell(&date)));
    }
    (
        [
            (header::CONTENT_TYPE, "text/csv; charset=utf-8"),
            (
                header::CONTENT_DISPOSITION,
                "attachment; filename=\"subscribers.csv\"",
            ),
        ],
        csv,
    )
        .into_response()
}

/// A value made safe for a CSV cell: quoted, and with a leading `'` when it
/// starts like a formula, so spreadsheet apps don't run it
/// (`=HYPERLINK(...)@example.com` would otherwise become a live formula).
fn csv_cell(value: &str) -> String {
    let value = value.replace(['\n', '\r'], " ");
    let value = if value.starts_with(['=', '+', '-', '@', '\t']) {
        format!("'{value}")
    } else {
        value
    };
    format!("\"{}\"", value.replace('"', "\"\""))
}

/// DELETE /admin/subscribers/:id -> Remove someone from the list entirely.
pub async fn delete(State(state): State<AppState>, Path(id): Path<i64>) -> StatusCode {
    match sqlx::query("DELETE FROM subscribers WHERE id = ?")
        .bind(id)
        .execute(&state.pool)
        .await
    {
        Ok(r) if r.rows_affected() > 0 => StatusCode::OK,
        Ok(_) => StatusCode::NOT_FOUND,
        Err(_) => StatusCode::INTERNAL_SERVER_ERROR,
    }
}

#[cfg(test)]
mod tests {
    use super::csv_cell;

    #[test]
    fn csv_cells_cannot_become_formulas() {
        assert_eq!(csv_cell("a@example.com"), "\"a@example.com\"");
        assert_eq!(
            csv_cell("=HYPERLINK(\"x\")@evil.com"),
            "\"'=HYPERLINK(\"\"x\"\")@evil.com\""
        );
        assert_eq!(csv_cell("+1@x.com"), "\"'+1@x.com\"");
    }
}
