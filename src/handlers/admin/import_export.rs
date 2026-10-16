//! Settings → Import & export: bringing a site in from WordPress and taking
//! posts out. Admins only. The work is in `services/import/` and
//! `services/export.rs`.

use super::UploadProblem;
use crate::app::models::CurrentUser;
use crate::app::security::log_event;
use crate::app::state::AppState;
use crate::services::import::{self, wordpress, Progress, Report, Summary, View};

use askama::Template;
use axum::body::Body;
use axum::extract::{Extension, Multipart, State};
use axum::http::{header, HeaderValue, StatusCode};
use axum::response::{IntoResponse, Response};
use axum_extra::extract::Form;
use serde::Deserialize;

/// The Import card: the upload form, a checked file, the progress or the
/// report, whichever the import is at.
#[derive(Default)]
pub struct ImportCard {
    pub checked: Option<Summary>,
    pub running: Option<Progress>,
    pub done: Option<Report>,
    /// A message for the upload form (English; the template translates it).
    pub error: Option<&'static str>,
}

impl ImportCard {
    /// The card for where the import is now.
    pub fn now() -> Self {
        match import::now() {
            View::Idle => Self::default(),
            View::Checked(summary) => Self {
                checked: Some(summary),
                ..Self::default()
            },
            View::Running(progress) => Self {
                running: Some(progress),
                ..Self::default()
            },
            View::Done(report) => Self {
                done: Some(report),
                ..Self::default()
            },
        }
    }

    fn with_error(error: &'static str) -> Self {
        Self {
            error: Some(error),
            ..Self::now()
        }
    }
}

/// The Import card alone: swapped in after each step, and asked for every
/// second while an import runs (to move the progress along).
#[derive(Template)]
#[template(path = "import.html")]
pub struct ImportTemplate {
    pub me: CurrentUser,
    pub import: ImportCard,
}

/// GET /admin/import -> The Import card as it is now.
pub async fn card(Extension(me): Extension<CurrentUser>) -> Response {
    ImportTemplate {
        me,
        import: ImportCard::now(),
    }
    .into_response()
}

/// POST /admin/import/check -> Read an uploaded WordPress export and show
/// what's in it. Nothing on the site changes yet.
pub async fn check(
    State(state): State<AppState>,
    Extension(me): Extension<CurrentUser>,
    multipart: Multipart,
) -> Response {
    let file = state.config.folder.join("data").join("import-upload.xml");
    let received = super::save_upload(multipart, "file", &file, import::MAX_FILE_BYTES)
        .await
        .map_err(|problem| match problem {
            UploadProblem::TooLarge => "The file is too large to import (the limit is 200 MB).",
            UploadProblem::Interrupted => "The upload didn't finish. Please try again.",
            UploadProblem::NoSpace => {
                "There isn't enough free disk space on the server to read this file."
            }
            UploadProblem::Missing => wordpress::NOT_WORDPRESS,
        });
    let checked = match received {
        Ok(()) => import::check(&state, &file).await,
        Err(message) => Err(message),
    };
    let _ = tokio::fs::remove_file(&file).await;
    let import = match checked {
        Ok(()) => ImportCard::now(),
        Err(message) => ImportCard::with_error(message),
    };
    ImportTemplate { me, import }.into_response()
}

#[derive(Deserialize)]
pub struct StartForm {
    /// Ticked boxes are sent; unticked ones aren't.
    #[serde(default)]
    copy_images: Option<String>,
    #[serde(default)]
    bring_comments: Option<String>,
}

/// POST /admin/import/start -> Save a backup, then import the checked file
/// in the background. The card follows its progress.
pub async fn start(
    State(state): State<AppState>,
    Extension(me): Extension<CurrentUser>,
    Form(form): Form<StartForm>,
) -> Response {
    let copy_images = form.copy_images.is_some();
    let bring_comments = form.bring_comments.is_some();
    let import = match import::start(state, me.id, copy_images, bring_comments) {
        Ok(()) => {
            log_event("import_started", &[("user", &me.email)]);
            ImportCard::now()
        }
        Err(message) => ImportCard::with_error(message),
    };
    ImportTemplate { me, import }.into_response()
}

/// POST /admin/import/reset -> Forget a checked file or a finished import,
/// back to the upload form.
pub async fn reset(Extension(me): Extension<CurrentUser>) -> Response {
    import::reset();
    ImportTemplate {
        me,
        import: ImportCard::now(),
    }
    .into_response()
}

// ---- Export ----

/// A file to save, named after the site and today's date.
fn download(body: Body, content_type: &'static str, ending: &str) -> Response {
    let name = format!(
        "attachment; filename=\"bloogla-export-{}.{ending}\"",
        chrono::Utc::now().format("%Y-%m-%d")
    );
    let mut response = body.into_response();
    let headers = response.headers_mut();
    headers.insert(header::CONTENT_TYPE, HeaderValue::from_static(content_type));
    headers.insert(header::CACHE_CONTROL, HeaderValue::from_static("no-store"));
    if let Ok(value) = HeaderValue::from_str(&name) {
        headers.insert(header::CONTENT_DISPOSITION, value);
    }
    response
}

/// GET /admin/export/wordpress.xml -> Posts and pages as a WordPress file.
pub async fn export_wordpress(
    State(state): State<AppState>,
    Extension(me): Extension<CurrentUser>,
) -> Response {
    let xml = crate::services::export::wordpress_file(&state).await;
    log_event(
        "export_downloaded",
        &[("user", &me.email), ("kind", "wordpress")],
    );
    download(Body::from(xml), "application/xml", "xml")
}

/// GET /admin/export/markdown.zip -> Posts and pages as Markdown files, with
/// their images, in a .zip.
pub async fn export_markdown(
    State(state): State<AppState>,
    Extension(me): Extension<CurrentUser>,
) -> Response {
    let archive = crate::services::backup::timestamped_path("export-").with_extension("zip");
    let written = match archive.parent().map(std::fs::create_dir_all) {
        Some(Err(e)) => Err(e.to_string()),
        _ => crate::services::export::write_markdown_archive(&state, &archive).await,
    };
    let file = match written {
        Ok(()) => tokio::fs::File::open(&archive)
            .await
            .map_err(|e| e.to_string()),
        Err(e) => Err(e),
    };
    // The open file keeps its contents, so the name can go right away and no
    // copy is left behind on the server.
    let _ = std::fs::remove_file(&archive);
    match file {
        Ok(file) => {
            log_event(
                "export_downloaded",
                &[("user", &me.email), ("kind", "markdown")],
            );
            let body = Body::from_stream(tokio_util::io::ReaderStream::new(file));
            download(body, "application/zip", "zip")
        }
        Err(e) => {
            tracing::error!("Markdown export failed: {e}");
            let message = super::alert(
                me.lang,
                "error",
                "Couldn't make the export. Check that there's enough free disk space, then try again.",
            );
            (StatusCode::INTERNAL_SERVER_ERROR, message).into_response()
        }
    }
}
