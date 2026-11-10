//! Settings → Backup: downloading a backup of the whole site, the daily
//! copy on the server, and restoring a backup. Admins only. The work is done
//! in `services/backup.rs` and `services/restore.rs`.

use super::{alert, UploadProblem};
use crate::app::models::CurrentUser;
use crate::app::security::log_event;
use crate::app::state::AppState;

use crate::db::settings;
use crate::services::restore;
use askama::Template;
use axum::body::Body;
use axum::extract::{Extension, Multipart, State};
use axum::http::{header, HeaderValue, StatusCode};
use axum::response::{IntoResponse, Response};
use axum_extra::extract::Form;
use serde::Deserialize;

/// Shown instead of the download when something went wrong.
fn failed(me: &CurrentUser) -> Response {
    let message = alert(
        me.lang,
        "error",
        "Couldn't make the backup. Please try again.",
    );
    (StatusCode::INTERNAL_SERVER_ERROR, message).into_response()
}

/// GET /admin/backup.zip -> The database and every uploaded file, as one .zip.
pub async fn download(
    State(state): State<AppState>,
    Extension(me): Extension<CurrentUser>,
) -> Response {
    let uploads = state
        .config
        .folder
        .join(crate::services::images::UPLOAD_DIR);
    let archive = match crate::services::backup::download_archive(&state.pool, &uploads).await {
        Ok(path) => path,
        Err(e) => {
            tracing::error!("Backup download failed: {e}");
            return failed(&me);
        }
    };
    let file = match tokio::fs::File::open(&archive).await {
        Ok(file) => file,
        Err(e) => {
            tracing::error!("Backup download failed: {e}");
            return failed(&me);
        }
    };
    // The open file keeps its contents, so the name can go right away and no
    // copy is left behind on the server.
    let _ = std::fs::remove_file(&archive);
    log_event("backup_downloaded", &[("user", &me.email)]);

    let name = format!(
        "attachment; filename=\"bloogla-backup-{}.zip\"",
        chrono::Utc::now().format("%Y-%m-%d")
    );
    let mut response = Body::from_stream(tokio_util::io::ReaderStream::new(file)).into_response();
    let headers = response.headers_mut();
    headers.insert(
        header::CONTENT_TYPE,
        HeaderValue::from_static("application/zip"),
    );
    headers.insert(header::CACHE_CONTROL, HeaderValue::from_static("no-store"));
    if let Ok(value) = HeaderValue::from_str(&name) {
        headers.insert(header::CONTENT_DISPOSITION, value);
    }
    response
}

#[derive(Deserialize)]
pub struct BackupSettingsForm {
    auto_backup: Option<String>,
}

/// POST /admin/settings/backup -> Save the "every day" checkbox.
pub async fn save_settings(
    State(state): State<AppState>,
    Extension(me): Extension<CurrentUser>,
    Form(form): Form<BackupSettingsForm>,
) -> Response {
    // An unticked checkbox isn't sent at all.
    let values = [("auto_backup", form.auto_backup.is_some().to_string())];
    match settings::save(&state.pool, &values).await {
        Ok(()) => alert(me.lang, "success", "Saved."),
        Err(e) => {
            tracing::error!("Failed to save backup settings: {e}");
            alert(
                me.lang,
                "error",
                "Couldn't save your changes. Please try again.",
            )
        }
    }
}

/// What the "Copy to storage" card shows.
pub struct StorageCard {
    pub on: bool,
    pub endpoint: String,
    pub region: String,
    pub bucket: String,
    pub access_key: String,
    pub has_secret_key: bool,
    /// When the last copy arrived, in the site's time zone.
    pub last_copied: Option<String>,
    /// Why the last copy failed (English; the template translates it).
    pub last_problem: Option<String>,
}

impl StorageCard {
    pub fn new(site: &settings::Settings, me: &CurrentUser) -> Self {
        StorageCard {
            on: site.storage_on,
            endpoint: site.storage_endpoint.clone(),
            region: site.storage_region.clone(),
            bucket: site.storage_bucket.clone(),
            access_key: site.storage_access_key.clone(),
            has_secret_key: !site.storage_secret_key.is_empty(),
            last_copied: (!site.storage_last_copied.is_empty()).then(|| {
                crate::content::text::display_datetime(
                    me.lang,
                    site.timezone,
                    &site.storage_last_copied,
                )
            }),
            last_problem: (!site.storage_last_problem.is_empty())
                .then(|| site.storage_last_problem.clone()),
        }
    }
}

#[derive(Deserialize)]
pub struct StorageForm {
    storage_on: Option<String>,
    #[serde(default)]
    storage_endpoint: String,
    #[serde(default)]
    storage_region: String,
    #[serde(default)]
    storage_bucket: String,
    #[serde(default)]
    storage_access_key: String,
    /// Left empty to keep the saved key.
    #[serde(default)]
    storage_secret_key: String,
}

/// POST /admin/settings/storage -> Save where backups are copied, then
/// check it works by saving a small file there.
pub async fn save_storage(
    State(state): State<AppState>,
    Extension(me): Extension<CurrentUser>,
    Form(form): Form<StorageForm>,
) -> Response {
    let on = form.storage_on.is_some();
    let endpoint = form
        .storage_endpoint
        .trim()
        .trim_end_matches('/')
        .to_string();
    if on && !endpoint.starts_with("https://") {
        return (
            StatusCode::UNPROCESSABLE_ENTITY,
            alert(
                me.lang,
                "error",
                crate::services::storage::Problem::Address.message(),
            ),
        )
            .into_response();
    }
    let mut values = vec![
        ("storage_on", on.to_string()),
        ("storage_endpoint", endpoint),
        ("storage_region", form.storage_region.trim().to_string()),
        ("storage_bucket", form.storage_bucket.trim().to_string()),
        (
            "storage_access_key",
            form.storage_access_key.trim().to_string(),
        ),
    ];
    if !form.storage_secret_key.trim().is_empty() {
        values.push((
            "storage_secret_key",
            form.storage_secret_key.trim().to_string(),
        ));
    }
    if let Err(e) = settings::save(&state.pool, &values).await {
        tracing::error!("Failed to save storage settings: {e}");
        return alert(
            me.lang,
            "error",
            "Couldn't save your changes. Please try again.",
        );
    }
    log_event(
        "backup_storage_changed",
        &[("user", &me.email), ("on", &on.to_string())],
    );
    if !on {
        return alert(me.lang, "success", "Saved.");
    }

    let site = settings::load(&state.pool).await;
    let Some(storage) = crate::services::storage::Storage::from_settings(&site) else {
        return alert(
            me.lang,
            "error",
            "Saved. Fill in every field to start copying backups.",
        );
    };
    let checked = crate::services::storage::check(storage).await;
    // Remembered, so Site health on the Dashboard says it too.
    let problem = checked
        .as_ref()
        .err()
        .map_or("", |problem| problem.message());
    if let Err(e) = settings::save(&state.pool, &[("storage_last_problem", problem.into())]).await {
        tracing::error!("Could not save how the storage check went: {e}");
    }
    match checked {
        Ok(()) => alert(
            me.lang,
            "success",
            "Saved. Bloogla could save a file in your storage, so backups will be copied there every day.",
        ),
        Err(problem) => {
            tracing::warn!("Checking the backup storage failed: {problem:?}");
            alert(me.lang, "error", problem.message())
        }
    }
}

/// The Restore card: the upload form, or what a checked backup contains.
#[derive(Default)]
pub struct RestoreCard {
    pub backup: Option<CheckedBackup>,
    /// A message from [`restore`] (English; the template translates it).
    pub error: Option<&'static str>,
}

/// A checked backup, as the Restore card shows it.
pub struct CheckedBackup {
    pub posts: i64,
    pub pages: i64,
    pub people: i64,
    pub images: i64,
    /// Title and date of the newest post.
    pub newest_post: Option<(String, String)>,
}

/// The Restore card alone, swapped in after "Check backup" and "Cancel".
#[derive(Template)]
#[template(path = "restore.html")]
pub struct RestoreTemplate {
    pub me: CurrentUser,
    pub restore: RestoreCard,
}

/// One backup is checked or restored at a time.
static RESTORING: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

const BUSY: &str = "Another backup is being checked or restored. Wait a moment and try again.";

/// POST /admin/backup/restore/check -> Unpack and look at an uploaded
/// backup. Nothing on the site changes yet.
pub async fn check_restore(
    State(state): State<AppState>,
    Extension(me): Extension<CurrentUser>,
    multipart: Multipart,
) -> Response {
    let card = |restore| {
        RestoreTemplate {
            me: me.clone(),
            restore,
        }
        .into_response()
    };
    let Ok(_turn) = RESTORING.try_lock() else {
        return card(RestoreCard {
            error: Some(BUSY),
            ..RestoreCard::default()
        });
    };
    let folder = &state.config.folder;
    let upload = folder.join("data").join("restore-upload.zip");
    let checked = match receive_backup(multipart, &upload).await {
        Ok(()) => restore::check(folder, &upload).await,
        Err(message) => Err(message),
    };
    let _ = tokio::fs::remove_file(&upload).await;
    match checked {
        Ok(summary) => {
            let tz = settings::load(&state.pool).await.timezone;
            let newest_post = summary.newest_post.map(|(title, date)| {
                let date = crate::content::text::display_date(me.lang, tz, &date);
                (title, date)
            });
            card(RestoreCard {
                backup: Some(CheckedBackup {
                    posts: summary.posts,
                    pages: summary.pages,
                    people: summary.people,
                    images: summary.images,
                    newest_post,
                }),
                error: None,
            })
        }
        Err(message) => card(RestoreCard {
            error: Some(message),
            ..RestoreCard::default()
        }),
    }
}

/// Save the uploaded backup (the form field `backup`) at `path`.
async fn receive_backup(multipart: Multipart, path: &std::path::Path) -> Result<(), &'static str> {
    super::save_upload(multipart, "backup", path, restore::MAX_BACKUP_BYTES)
        .await
        .map_err(|problem| match problem {
            UploadProblem::TooLarge => restore::TOO_LARGE,
            UploadProblem::Interrupted => "The upload didn't finish. Please try again.",
            UploadProblem::NoSpace => restore::NO_SPACE,
            UploadProblem::Missing => restore::NOT_A_BACKUP,
        })
}

/// POST /admin/backup/restore/cancel -> Forget the checked backup.
pub async fn cancel_restore(
    State(state): State<AppState>,
    Extension(me): Extension<CurrentUser>,
) -> Response {
    if let Ok(_turn) = RESTORING.try_lock() {
        restore::cancel(&state.config.folder);
    }
    RestoreTemplate {
        me,
        restore: RestoreCard::default(),
    }
    .into_response()
}

/// POST /admin/backup/restore -> Save the site as it is now, then restart
/// with the checked backup in its place (admin.js waits and reloads).
pub async fn restore(
    State(state): State<AppState>,
    Extension(me): Extension<CurrentUser>,
) -> Response {
    let Ok(_turn) = RESTORING.try_lock() else {
        return alert(me.lang, "error", BUSY);
    };
    let folder = &state.config.folder;
    if !restore::is_checked(folder) {
        return alert(me.lang, "error", "Check the backup again first.");
    }
    let safety = match restore::get_ready(&state.pool, folder).await {
        Ok(path) => path,
        Err(e) => {
            tracing::error!("Restore stopped, the safety backup failed: {e}");
            return alert(
                me.lang,
                "error",
                "Couldn't save a backup of your site as it is now, so nothing was restored. Check that there's enough free disk space, then try again.",
            );
        }
    };
    log_event(
        "backup_restore_started",
        &[
            ("user", &me.email),
            ("saved_first", &safety.display().to_string()),
        ],
    );

    // Windows can't start Bloogla again by itself (see services/updates.rs).
    let program = std::env::current_exe().ok().filter(|_| cfg!(unix));
    let Some(program) = program else {
        return alert(
            me.lang,
            "success",
            "The backup is ready. Close Bloogla and open it again to finish restoring.",
        );
    };
    crate::services::updates::restart_soon(program);
    (
        [("HX-Trigger", "restarting")],
        alert(
            me.lang,
            "success",
            "Restoring the backup. Bloogla restarts and this page reloads; if it asks you to sign in, use an account from the backup.",
        ),
    )
        .into_response()
}
