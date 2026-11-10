//! Settings → Subscribers: letting readers get new posts by email, and the
//! list of people who subscribed. Admins only. The form under posts is in
//! `handlers/site/subscribe.rs`, sending in `services/subscriptions.rs`.

use super::alert;
use crate::app::models::CurrentUser;
use crate::app::security::log_event;
use crate::app::state::AppState;
use crate::content::text::display_date;
use crate::db::{settings, subscribers};

use axum::extract::{Extension, Path, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum_extra::extract::Form;
use serde::Deserialize;

/// What the Subscribers tab shows.
pub struct SubscribersCard {
    pub on: bool,
    /// Email is set up (subscriptions need it).
    pub email_ready: bool,
    pub rows: Vec<SubscriberRow>,
}

/// Someone on the list.
pub struct SubscriberRow {
    pub id: i64,
    pub email: String,
    /// The day they confirmed.
    pub since: String,
}

impl SubscribersCard {
    pub async fn load(state: &AppState, site: &settings::Settings, me: &CurrentUser) -> Self {
        let rows = subscribers::confirmed(&state.pool)
            .await
            .into_iter()
            .map(|subscriber| SubscriberRow {
                id: subscriber.id,
                since: display_date(me.lang, site.timezone, &subscriber.confirmed_at),
                email: subscriber.email,
            })
            .collect();
        SubscribersCard {
            on: site.subscriptions_on,
            email_ready: crate::services::email::smtp_settings(state).await.is_some(),
            rows,
        }
    }
}

#[derive(Deserialize)]
pub struct SubscribersForm {
    subscriptions_on: Option<String>,
}

/// POST /admin/settings/subscribers -> Turn the subscribe form on or off.
pub async fn save_settings(
    State(state): State<AppState>,
    Extension(me): Extension<CurrentUser>,
    Form(form): Form<SubscribersForm>,
) -> Response {
    let on = form.subscriptions_on.is_some();
    if on
        && crate::services::email::smtp_settings(&state)
            .await
            .is_none()
    {
        return (
            StatusCode::UNPROCESSABLE_ENTITY,
            alert(
                me.lang,
                "error",
                "Set up email first (Settings → Email): new posts are sent from it.",
            ),
        )
            .into_response();
    }
    match settings::save(&state.pool, &[("subscriptions_on", on.to_string())]).await {
        Ok(()) => alert(me.lang, "success", "Saved."),
        Err(e) => {
            tracing::error!("Failed to save subscription settings: {e}");
            alert(
                me.lang,
                "error",
                "Couldn't save your changes. Please try again.",
            )
        }
    }
}

/// DELETE /admin/subscribers/:id -> Take someone off the list.
pub async fn delete(
    State(state): State<AppState>,
    Extension(me): Extension<CurrentUser>,
    Path(id): Path<i64>,
) -> StatusCode {
    match subscribers::delete(&state.pool, id).await {
        Ok(true) => {
            log_event("subscriber_removed", &[("user", &me.email)]);
            StatusCode::OK
        }
        Ok(false) => StatusCode::NOT_FOUND,
        Err(e) => {
            tracing::error!("Failed to remove a subscriber: {e}");
            StatusCode::INTERNAL_SERVER_ERROR
        }
    }
}
