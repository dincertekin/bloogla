//! Downloading a backup of the whole site (Admin → Settings → Backup).
//! Admins only. The work is done in `services/backup.rs`.

use super::alert;
use crate::app::models::CurrentUser;
use crate::app::security::log_event;
use crate::app::state::AppState;

use axum::body::Body;
use axum::extract::{Extension, State};
use axum::http::{header, HeaderValue, StatusCode};
use axum::response::{IntoResponse, Response};

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
    let uploads = std::path::Path::new(super::media::UPLOAD_DIR);
    let archive = match crate::services::backup::download_archive(&state.pool, uploads).await {
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
