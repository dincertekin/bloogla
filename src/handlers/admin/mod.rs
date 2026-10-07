//! The admin panel. Each screen has a file here, with its Askama template
//! struct next to its handlers; the HTML is in `admin/templates/`.
//!
//! Who may open which screen is decided in `server/routes.rs`.

pub mod auth;
pub mod comments;
pub mod dashboard;
pub mod media;
pub mod posts;
pub mod profile;
pub mod settings;
pub mod setup;
pub mod subscribers;
pub mod tags;
pub mod themes;
pub mod users;

use crate::i18n::Lang;

use axum::http::StatusCode;
use axum::response::{Html, IntoResponse, Response};

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
