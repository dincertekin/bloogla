//! Settings → Updates: checking GitHub for a new version and installing it.
//! The work itself (asking GitHub, checking signatures, restarting) is in
//! `services/updates.rs`.

use super::alert;
use crate::app::models::CurrentUser;
use crate::app::security::log_event;
use crate::app::state::AppState;
use crate::content::text::{display_datetime, escape_html};
use crate::db::settings::{self, Settings};
use crate::services::updates::{self, InstallMethod, Release};

use askama::Template;
use axum::extract::{Extension, State};
use axum::response::{IntoResponse, Response};
use axum_extra::extract::Form;
use serde::Deserialize;

/// What Settings → Updates shows: the version, what the last check found,
/// and how this server can be updated.
pub struct UpdateStatus {
    pub current: &'static str,
    /// When updates were last checked, e.g. `Oct 9, 14:30`, or empty.
    pub last_checked: String,
    /// Checked since Bloogla started, so "You have the latest version" is true.
    pub checked: bool,
    pub newer: Option<Release>,
    pub error: Option<String>,
    pub method: InstallMethod,
    pub check_daily: bool,
    pub install_auto: bool,
}

impl UpdateStatus {
    pub fn load(
        state: &AppState,
        site: &Settings,
        me: &CurrentUser,
        error: Option<String>,
    ) -> Self {
        let newer = updates::available(&state.newer_release);
        Self {
            current: updates::CURRENT_VERSION,
            last_checked: if site.update_last_checked.is_empty() {
                String::new()
            } else {
                display_datetime(me.lang, site.timezone, &site.update_last_checked)
            },
            checked: updates::checked_since_start(),
            newer,
            error,
            method: updates::install_method(),
            check_daily: site.update_check_daily,
            install_auto: site.update_install_auto,
        }
    }

    /// This server can download the new version and restart by itself.
    pub fn can_install(&self) -> bool {
        self.method == InstallMethod::Itself
            && self.newer.as_ref().is_some_and(|r| r.download.is_some())
    }

    pub fn can_update_itself(&self) -> bool {
        self.method == InstallMethod::Itself
    }

    pub fn in_docker(&self) -> bool {
        self.method == InstallMethod::Docker
    }
}

/// The status box alone, swapped in after "Check for updates".
#[derive(Template)]
#[template(path = "update_status.html")]
pub struct UpdateStatusTemplate {
    pub me: CurrentUser,
    pub update: UpdateStatus,
}

/// POST /admin/updates/check -> Ask GitHub now.
pub async fn check(
    State(state): State<AppState>,
    Extension(me): Extension<CurrentUser>,
) -> Response {
    let error = match updates::check(&state).await {
        Ok(_) => None,
        Err(e) => {
            tracing::warn!("Could not check for updates: {e}");
            Some(me.tv("Couldn't reach GitHub: {error}", escape_html(&e)))
        }
    };
    let site = settings::load(&state.pool).await;
    let update = UpdateStatus::load(&state, &site, &me, error);
    UpdateStatusTemplate { me, update }.into_response()
}

/// POST /admin/updates/install -> Download, check and install the new
/// version, then restart (admin.js waits and reloads the page).
pub async fn install(
    State(state): State<AppState>,
    Extension(me): Extension<CurrentUser>,
) -> Response {
    let Some(release) = updates::available(&state.newer_release) else {
        return alert(me.lang, "error", "Check for updates first.");
    };
    if updates::install_method() != InstallMethod::Itself {
        return alert(
            me.lang,
            "error",
            "This server can't update Bloogla by itself.",
        );
    }
    match updates::install(&release).await {
        Ok(()) => {
            log_event(
                "update_installed",
                &[("version", &release.version), ("by", &me.email)],
            );
            (
                [("HX-Trigger", "restarting")],
                alert(
                    me.lang,
                    "success",
                    &me.tv(
                        "Bloogla {version} is installed. Restarting; this page reloads in a moment.",
                        &release.version,
                    ),
                ),
            )
                .into_response()
        }
        Err(e) => alert(
            me.lang,
            "error",
            &me.tv(
                "Couldn't install the update: {error}",
                escape_html(&me.lang.t_owned(&e)),
            ),
        ),
    }
}

#[derive(Deserialize)]
pub struct UpdateSettingsForm {
    check_daily: Option<String>,
    install_auto: Option<String>,
}

/// POST /admin/settings/updates -> Save the two checkboxes.
pub async fn save_settings(
    State(state): State<AppState>,
    Extension(me): Extension<CurrentUser>,
    Form(form): Form<UpdateSettingsForm>,
) -> Response {
    let check_daily = form.check_daily.is_some();
    // Installing automatically needs the daily check to find something.
    let install_auto = check_daily && form.install_auto.is_some();
    let values = [
        ("update_check_daily", check_daily.to_string()),
        ("update_install_auto", install_auto.to_string()),
    ];
    match settings::save(&state.pool, &values).await {
        Ok(()) => alert(me.lang, "success", "Saved."),
        Err(e) => {
            tracing::error!("Failed to save update settings: {e}");
            alert(
                me.lang,
                "error",
                "Couldn't save your changes. Please try again.",
            )
        }
    }
}
