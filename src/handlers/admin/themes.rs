//! The themes page: choose how the site looks, upload themes as .zip files
//! and delete uploaded ones. Admins only.

use crate::app::models::CurrentUser;
use crate::app::security::log_event;
use crate::app::state::AppState;
use crate::db::settings;
use crate::services::themes::{self, ThemeInfo};

use askama::Template;
use axum::extract::{Extension, Multipart, Path, Query, State};
use axum::http::StatusCode;
use axum::response::{Html, IntoResponse, Redirect, Response};
use axum_extra::extract::Form;
use serde::Deserialize;

#[derive(Template)]
#[template(path = "themes.html")]
pub struct ThemesTemplate {
    pub blog_name: String,
    pub me: CurrentUser,
    pub active_theme: String,
    pub themes: Vec<ThemeInfo>,
    /// A message after an upload or a change: `(kind, text)`, where kind is
    /// `success` or `error`.
    pub message: Option<(&'static str, String)>,
}

/// What happened, from the address after a redirect (`?uploaded=my-theme`).
#[derive(Deserialize)]
pub struct Done {
    uploaded: Option<String>,
    activated: Option<String>,
}

#[derive(Deserialize)]
pub struct ActivateForm {
    theme: String,
}

async fn page_with(
    state: &AppState,
    me: CurrentUser,
    message: Option<(&'static str, String)>,
) -> ThemesTemplate {
    let site = settings::load(&state.pool).await;
    let themes = state
        .themes
        .read()
        .map(|themes| themes.list())
        .unwrap_or_default();
    ThemesTemplate {
        blog_name: site.blog_name.clone(),
        me,
        active_theme: site.active_theme.clone(),
        themes,
        message,
    }
}

/// The name of an installed theme, for messages.
fn theme_name(state: &AppState, id: &str) -> String {
    state
        .themes
        .read()
        .ok()
        .and_then(|themes| themes.list().into_iter().find(|t| t.id == id))
        .map_or_else(|| id.to_string(), |t| t.meta.name)
}

/// GET /admin/themes -> Installed themes.
pub async fn page(
    State(state): State<AppState>,
    Extension(me): Extension<CurrentUser>,
    Query(done): Query<Done>,
) -> impl IntoResponse {
    let message = if let Some(id) = done.uploaded {
        Some((
            "success",
            me.tv("Theme uploaded: {name}.", theme_name(&state, &id)),
        ))
    } else {
        done.activated.map(|id| {
            (
                "success",
                me.tv("Your site now uses {name}.", theme_name(&state, &id)),
            )
        })
    };
    page_with(&state, me, message).await
}

/// POST /admin/themes/activate -> Use another theme for the site.
pub async fn activate(
    State(state): State<AppState>,
    Extension(me): Extension<CurrentUser>,
    Form(form): Form<ActivateForm>,
) -> Response {
    let usable = state
        .themes
        .read()
        .is_ok_and(|themes| themes.usable(&form.theme).is_some());
    if !usable {
        let message = me.t("This theme can't be used.").to_string();
        return (
            StatusCode::UNPROCESSABLE_ENTITY,
            page_with(&state, me, Some(("error", message))).await,
        )
            .into_response();
    }
    if let Err(e) = settings::save(&state.pool, &[("active_theme", form.theme.clone())]).await {
        tracing::error!("Failed to save theme: {e}");
    }
    Redirect::to(&format!("/admin/themes?activated={}", form.theme)).into_response()
}

/// POST /admin/themes/upload -> Install a theme from a .zip file.
pub async fn upload(
    State(state): State<AppState>,
    Extension(me): Extension<CurrentUser>,
    mut multipart: Multipart,
) -> Response {
    let mut upload = None;
    while let Ok(Some(field)) = multipart.next_field().await {
        if field.name() != Some("theme") {
            continue;
        }
        let file_name = field.file_name().unwrap_or("theme.zip").to_string();
        if let Ok(data) = field.bytes().await {
            upload = Some((file_name, data));
        }
    }

    let result = match upload {
        Some((file_name, data)) if !data.is_empty() => {
            let name = file_name.clone();
            tokio::task::spawn_blocking(move || themes::install_zip(&data, &name))
                .await
                .unwrap_or_else(|_| Err("Couldn't save the theme. Please try again.".into()))
                .map(|id| (file_name, id))
        }
        _ => Err("Choose a .zip file to upload.".into()),
    };

    match result {
        Ok((file_name, id)) => {
            if let Ok(mut themes) = state.themes.write() {
                themes.reload(&id);
            }
            log_event(
                "theme_uploaded",
                &[("user", &me.email), ("theme", &id), ("file", &file_name)],
            );
            Redirect::to(&format!("/admin/themes?uploaded={id}")).into_response()
        }
        Err(error) => {
            let message = me.lang.t_owned(&error);
            (
                StatusCode::UNPROCESSABLE_ENTITY,
                page_with(&state, me, Some(("error", message))).await,
            )
                .into_response()
        }
    }
}

/// DELETE /admin/themes/:id -> Delete an uploaded theme that isn't in use.
pub async fn delete(
    State(state): State<AppState>,
    Extension(me): Extension<CurrentUser>,
    Path(id): Path<String>,
) -> Response {
    if settings::load(&state.pool).await.active_theme == id {
        return (
            StatusCode::CONFLICT,
            me.t("Switch to another theme before deleting this one."),
        )
            .into_response();
    }
    match themes::delete(&id) {
        Ok(()) => {
            if let Ok(mut themes) = state.themes.write() {
                themes.reload(&id);
            }
            log_event("theme_deleted", &[("user", &me.email), ("theme", &id)]);
            Html("").into_response()
        }
        Err(error) => (StatusCode::UNPROCESSABLE_ENTITY, me.t(error)).into_response(),
    }
}
