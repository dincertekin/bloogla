//! The admin panel. Each screen has a file here, with its Askama template
//! struct next to its handlers; the HTML is in `admin/templates/`.
//!
//! Who may open which screen is decided in `server/routes.rs`.

pub mod auth;
pub mod backup;
pub mod comments;
pub mod dashboard;
pub mod import_export;
pub mod media;
pub mod messages;
pub mod password_reset;
pub mod posts;
pub mod profile;
pub mod settings;
pub mod setup;
pub mod tags;
pub mod themes;
pub mod updates;
pub mod users;

use crate::i18n::Lang;

use axum::http::StatusCode;
use axum::response::{Html, IntoResponse, Response};

/// Why an upload saved with [`save_upload`] didn't arrive.
#[derive(Debug, PartialEq)]
pub enum UploadProblem {
    /// Over the size limit.
    TooLarge,
    /// Cut short (the connection dropped).
    Interrupted,
    /// Couldn't be written, usually a full disk.
    NoSpace,
    /// The form had no file in the field.
    Missing,
}

/// Save the file sent in the form field `field` at `path`, a piece at a time,
/// so a large file never has to fit in memory.
pub async fn save_upload(
    mut multipart: axum::extract::Multipart,
    field: &str,
    path: &std::path::Path,
    max_bytes: u64,
) -> Result<(), UploadProblem> {
    use tokio::io::AsyncWriteExt;

    let interrupted = |e: axum::extract::multipart::MultipartError| {
        if e.status() == StatusCode::PAYLOAD_TOO_LARGE {
            UploadProblem::TooLarge
        } else {
            UploadProblem::Interrupted
        }
    };
    if let Some(parent) = path.parent() {
        tokio::fs::create_dir_all(parent)
            .await
            .map_err(|_| UploadProblem::NoSpace)?;
    }
    while let Some(mut upload) = multipart.next_field().await.map_err(interrupted)? {
        if upload.name() != Some(field) {
            continue;
        }
        let mut file = tokio::fs::File::create(path)
            .await
            .map_err(|_| UploadProblem::NoSpace)?;
        let mut size = 0u64;
        while let Some(chunk) = upload.chunk().await.map_err(interrupted)? {
            size += chunk.len() as u64;
            if size > max_bytes {
                return Err(UploadProblem::TooLarge);
            }
            file.write_all(&chunk)
                .await
                .map_err(|_| UploadProblem::NoSpace)?;
        }
        file.flush().await.map_err(|_| UploadProblem::NoSpace)?;
        return Ok(());
    }
    Err(UploadProblem::Missing)
}

/// Lets a handler return an admin template as its answer: the template is
/// turned into HTML, or a plain 500 page if that fails (it only can when a
/// template has a bug). The askama_axum crate used to write this for us,
/// but it only works with older versions of axum.
fn render_template(template: &impl askama::Template) -> Response {
    match template.render() {
        Ok(html) => Html(html).into_response(),
        Err(err) => {
            tracing::error!("admin template failed: {err}");
            StatusCode::INTERNAL_SERVER_ERROR.into_response()
        }
    }
}

/// Writes the `IntoResponse` code for each listed template.
macro_rules! templates_answer_requests {
    ($($template:ty),* $(,)?) => {
        $(
            impl IntoResponse for $template {
                fn into_response(self) -> Response {
                    render_template(&self)
                }
            }
        )*
    };
}

templates_answer_requests!(
    auth::LoginTemplate,
    auth::LoginCodeTemplate,
    backup::RestoreTemplate,
    comments::CommentsTemplate,
    dashboard::DashboardTemplate,
    import_export::ImportTemplate,
    media::MediaTemplate,
    media::MediaPickerTemplate,
    messages::MessagesTemplate,
    password_reset::ForgotTemplate,
    password_reset::ResetTemplate,
    posts::PostsTemplate,
    posts::PostEditorTemplate,
    profile::ProfileTemplate,
    profile::TwoFactorTemplate,
    settings::SettingsTemplate,
    setup::SetupTemplate,
    tags::TagsTemplate,
    tags::TagItemTemplate,
    themes::ThemesTemplate,
    themes::OptionsTemplate,
    updates::UpdateStatusTemplate,
    users::UsersTemplate,
    users::UserItemTemplate,
);

/// A status tab above a list, e.g. "Drafts (2)".
pub struct StatusFilter {
    pub value: &'static str,
    pub label: &'static str,
    pub count: usize,
}

impl StatusFilter {
    pub fn new(value: &'static str, label: &'static str, count: usize) -> Self {
        Self {
            value,
            label,
            count,
        }
    }
}

/// A translated message box for HTMX to put on the page.
/// `kind` is `success`, `error` or `info`.
pub fn alert(lang: Lang, kind: &str, message: &str) -> Response {
    Html(format!(
        r#"<div class="alert alert-{kind}">{}</div>"#,
        lang.t_owned(message)
    ))
    .into_response()
}

/// "Saved." for a change that affects the whole page, like its language:
/// admin.js reloads the page and shows the message again. `lang` is the
/// language the page will be in after reloading.
pub fn saved_and_reload(lang: Lang) -> Response {
    (
        [("HX-Trigger", "reload-page")],
        alert(lang, "success", "Saved."),
    )
        .into_response()
}

/// Plain 403 page for actions this person's role doesn't allow.
pub fn forbidden(lang: Lang) -> Response {
    (
        StatusCode::FORBIDDEN,
        Html(format!(
            r#"<!doctype html><html lang="{}"><meta charset="utf-8"><link rel="stylesheet" href="/static/css/style.css">
<main class="container" style="margin-top: 12vh"><div class="card">
<h1 style="font-size: 1.25rem; margin-bottom: .5rem">{}</h1>
<p class="text-secondary" style="margin-bottom: 1rem">{}</p>
<a href="/admin" class="btn btn-secondary">{}</a></div></main></html>"#,
            lang.code(),
            lang.t("You can't do that"),
            lang.t("Your role doesn't include this. Ask an admin if you need access."),
            lang.t("Back to dashboard"),
        )),
    )
        .into_response()
}
