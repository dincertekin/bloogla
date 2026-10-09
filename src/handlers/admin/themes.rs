//! The themes page: choose how the site looks and change a theme's options
//! (Customize). Admins only.

use crate::app::models::CurrentUser;
use crate::app::state::AppState;
use crate::db::settings;
use crate::services::themes::options::{self, ThemeOption};
use crate::services::themes::ThemeInfo;

use askama::Template;
use axum::extract::{Extension, Path, Query, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Redirect, Response};
use axum_extra::extract::Form;
use serde::Deserialize;
use std::collections::HashMap;

#[derive(Template)]
#[template(path = "themes.html")]
pub struct ThemesTemplate {
    pub blog_name: String,
    pub me: CurrentUser,
    pub active_theme: String,
    pub themes: Vec<ThemeInfo>,
    /// A message after a change: `(kind, text)`, where kind is
    /// `success` or `error`.
    pub message: Option<(&'static str, String)>,
}

/// What happened, from the address after a redirect (`?activated=docs`).
#[derive(Deserialize)]
pub struct Done {
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

/// How many themes are installed. With just one there's nothing to choose,
/// so "Appearance" goes straight to its options.
fn theme_count(state: &AppState) -> usize {
    state.themes.read().map_or(0, |themes| themes.list().len())
}

/// GET /admin/themes -> "Appearance": the active theme's options, or the
/// installed themes to choose from when there's more than one.
pub async fn page(
    State(state): State<AppState>,
    Extension(me): Extension<CurrentUser>,
    Query(done): Query<Done>,
) -> Response {
    if theme_count(&state) <= 1 && done.activated.is_none() {
        let active = settings::load(&state.pool).await.active_theme.clone();
        return Redirect::to(&format!("/admin/themes/{active}/options")).into_response();
    }
    let message = done.activated.map(|id| {
        (
            "success",
            me.tv("Your site now uses {name}.", theme_name(&state, &id)),
        )
    });
    page_with(&state, me, message).await.into_response()
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

#[derive(Template)]
#[template(path = "theme_options.html")]
pub struct OptionsTemplate {
    pub blog_name: String,
    pub me: CurrentUser,
    pub theme: ThemeInfo,
    /// Each option with its current value.
    pub fields: Vec<OptionField>,
    pub message: Option<(&'static str, String)>,
    /// True when other themes are installed (then there's a link to them).
    pub other_themes: bool,
}

pub struct OptionField {
    pub option: ThemeOption,
    pub value: String,
    pub error: Option<&'static str>,
}

#[derive(Deserialize)]
pub struct Saved {
    saved: Option<String>,
}

/// An installed theme by id.
fn find_theme(state: &AppState, id: &str) -> Option<ThemeInfo> {
    let themes = state.themes.read().ok()?;
    themes.list().into_iter().find(|t| t.id == id)
}

/// GET /admin/themes/:id/options -> The theme's options.
pub async fn options_page(
    State(state): State<AppState>,
    Extension(me): Extension<CurrentUser>,
    Path(id): Path<String>,
    Query(saved): Query<Saved>,
) -> Response {
    let Some(theme) = find_theme(&state, &id) else {
        return Redirect::to("/admin/themes").into_response();
    };
    let site = settings::load(&state.pool).await;
    let fields = theme
        .meta
        .options
        .iter()
        .map(|option| OptionField {
            value: options::current(&theme.id, option, &site.theme_options),
            option: option.clone(),
            error: None,
        })
        .collect();
    let message = saved.saved.map(|_| ("success", me.t("Saved.").to_string()));
    OptionsTemplate {
        blog_name: site.blog_name.clone(),
        me,
        theme,
        fields,
        message,
        other_themes: theme_count(&state) > 1,
    }
    .into_response()
}

/// POST /admin/themes/:id/options -> Save the theme's options.
pub async fn save_options(
    State(state): State<AppState>,
    Extension(me): Extension<CurrentUser>,
    Path(id): Path<String>,
    Form(form): Form<HashMap<String, String>>,
) -> Response {
    let Some(theme) = find_theme(&state, &id) else {
        return Redirect::to("/admin/themes").into_response();
    };

    // Check every value; show the form again if any is wrong.
    let fields: Vec<OptionField> = theme
        .meta
        .options
        .iter()
        .map(|option| {
            // Unticked checkboxes aren't sent at all.
            let typed = form.get(&option.name).map_or("", String::as_str);
            match option.clean(typed) {
                Ok(value) => OptionField {
                    option: option.clone(),
                    value,
                    error: None,
                },
                Err(error) => OptionField {
                    option: option.clone(),
                    value: typed.to_string(),
                    error: Some(error),
                },
            }
        })
        .collect();

    if fields.iter().any(|f| f.error.is_some()) {
        let message = me.t("Some values need fixing.").to_string();
        return (
            StatusCode::UNPROCESSABLE_ENTITY,
            OptionsTemplate {
                blog_name: settings::load(&state.pool).await.blog_name.clone(),
                me,
                theme,
                fields,
                message: Some(("error", message)),
                other_themes: theme_count(&state) > 1,
            },
        )
            .into_response();
    }

    let keys: Vec<String> = fields
        .iter()
        .map(|f| options::storage_key(&theme.id, &f.option.name))
        .collect();
    let values: Vec<(&str, String)> = keys
        .iter()
        .zip(&fields)
        .map(|(key, f)| (key.as_str(), f.value.clone()))
        .collect();
    if let Err(e) = settings::save(&state.pool, &values).await {
        tracing::error!("Failed to save theme options: {e}");
        return (
            StatusCode::INTERNAL_SERVER_ERROR,
            me.t("Couldn't save your changes. Please try again."),
        )
            .into_response();
    }
    Redirect::to(&format!("/admin/themes/{}/options?saved=1", theme.id)).into_response()
}
