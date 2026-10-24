//! Settings → Redirects: send visitors from an old address to a new one,
//! for links that still point to a site's old addresses after a move.
//! (When a post's own address changes, Bloogla adds the redirect by itself.)
//!
//! Redirects are only looked up when nothing else is at an address, in
//! `server/middleware.rs` (`redirects`), so they never hide a post or page.

use crate::app::models::CurrentUser;
use crate::app::state::AppState;
use crate::content::text::url_decode;
use crate::db::redirects;
use crate::i18n::Lang;

use askama::Template;
use axum::extract::{Extension, Path, State};
use axum::http::StatusCode;
use axum::response::{Html, IntoResponse, Response};
use axum_extra::extract::Form;
use serde::Deserialize;

/// Longest address accepted.
const MAX_ADDRESS_CHARS: usize = 500;

/// A redirect in the list, ready to show.
pub struct RedirectRow {
    pub id: i64,
    pub from_path: String,
    /// The new address, decoded so it reads like it was typed.
    pub to_address: String,
}

impl From<redirects::Redirect> for RedirectRow {
    fn from(redirect: redirects::Redirect) -> Self {
        Self {
            id: redirect.id,
            from_path: redirect.from_path,
            to_address: url_decode(&redirect.to_address),
        }
    }
}

/// One row, sent back after adding a redirect.
#[derive(Template)]
#[template(path = "redirect_item.html")]
pub struct RedirectItemTemplate {
    pub me: CurrentUser,
    pub row: RedirectRow,
}

/// Every redirect, for the Settings tab.
pub async fn rows(state: &AppState) -> Vec<RedirectRow> {
    redirects::all(&state.pool)
        .await
        .into_iter()
        .map(RedirectRow::from)
        .collect()
}

#[derive(Deserialize)]
pub struct RedirectForm {
    from: String,
    to: String,
}

/// POST /admin/redirects -> Add a redirect.
pub async fn create(
    State(state): State<AppState>,
    Extension(me): Extension<CurrentUser>,
    Form(form): Form<RedirectForm>,
) -> Response {
    let Some(from_path) = old_address(&form.from) else {
        return form_error(
            me.lang,
            "Enter the old address as it starts after your domain, like /old-page.",
        );
    };
    let Some(to_address) = new_address(&form.to) else {
        return form_error(
            me.lang,
            "Enter the new address, like /about or https://example.com/page.",
        );
    };
    if old_address(&to_address).as_deref() == Some(from_path.as_str()) {
        return form_error(
            me.lang,
            "The new address must be different from the old one.",
        );
    }

    let id = match redirects::add(&state.pool, &from_path, &to_address).await {
        Ok(Some(id)) => id,
        Ok(None) => {
            return form_error(
                me.lang,
                "This old address already leads somewhere. Delete that redirect first.",
            );
        }
        Err(e) => {
            tracing::error!("Failed to save a redirect: {e}");
            return form_error(me.lang, "Couldn't save. Please try again.");
        }
    };

    RedirectItemTemplate {
        me,
        row: RedirectRow {
            id,
            from_path,
            to_address: url_decode(&to_address),
        },
    }
    .into_response()
}

/// DELETE /admin/redirects/:id -> Remove a redirect.
pub async fn delete(State(state): State<AppState>, Path(id): Path<i64>) -> StatusCode {
    match redirects::delete(&state.pool, id).await {
        Ok(true) => StatusCode::OK,
        Ok(false) => StatusCode::NOT_FOUND,
        Err(e) => {
            tracing::error!("Failed to delete a redirect: {e}");
            StatusCode::INTERNAL_SERVER_ERROR
        }
    }
}

fn form_error(lang: Lang, message: &str) -> Response {
    (
        [
            ("HX-Retarget", "#redirect-form-alert"),
            ("HX-Reswap", "innerHTML"),
        ],
        Html(format!(
            r#"<div class="alert alert-error">{}</div>"#,
            lang.t_owned(message)
        )),
    )
        .into_response()
}

/// An old address as it's stored and looked up: the path only, decoded,
/// without a slash at the end. People may paste a whole link
/// (`https://old-site.com/old-page/?ref=x` → `/old-page`). `None` for
/// something that isn't an address, the home page or the admin panel.
pub fn old_address(input: &str) -> Option<String> {
    let input = input.trim();
    let path = match input.split_once("://") {
        Some((_, rest)) => rest.find('/').map_or("/", |at| &rest[at..]),
        None => input,
    };
    let path = path.split(['?', '#']).next().unwrap_or_default();
    let path = url_decode(path);
    let path = path.trim_end_matches('/');
    let valid = path.starts_with('/')
        && !path.starts_with("//")
        && path != "/admin"
        && !path.starts_with("/admin/")
        && path.chars().count() <= MAX_ADDRESS_CHARS
        && !path.chars().any(char::is_control);
    valid.then(|| path.to_string())
}

/// A new address as it's stored: an address on this site (`/about`) or a
/// whole link to another site, with spaces and letters like `ı` encoded so
/// browsers accept it. `None` for anything else.
fn new_address(input: &str) -> Option<String> {
    let input = input.trim();
    let valid = (input.starts_with('/') && !input.starts_with("//"))
        || input.starts_with("https://")
        || input.starts_with("http://");
    if !valid || input.chars().count() > MAX_ADDRESS_CHARS || input.chars().any(char::is_control) {
        return None;
    }
    let mut encoded = String::with_capacity(input.len());
    for c in input.chars() {
        if c.is_ascii_graphic() {
            encoded.push(c);
        } else {
            let mut bytes = [0; 4];
            for byte in c.encode_utf8(&mut bytes).bytes() {
                encoded.push_str(&format!("%{byte:02X}"));
            }
        }
    }
    Some(encoded)
}

#[cfg(test)]
mod tests {
    use super::{new_address, old_address};

    #[test]
    fn old_addresses_are_paths_without_extras() {
        assert_eq!(old_address(" /old-page/ "), Some("/old-page".into()));
        assert_eq!(
            old_address("https://old.example.com/2019/05/cake/?ref=x#top"),
            Some("/2019/05/cake".into())
        );
        assert_eq!(
            old_address("/hakk%C4%B1m%C4%B1zda"),
            Some("/hakkımızda".into())
        );
        for refused in [
            "",
            "/",
            "old-page",
            "//evil.example",
            "/admin",
            "/admin/posts",
        ] {
            assert_eq!(old_address(refused), None, "{refused}");
        }
    }

    #[test]
    fn new_addresses_are_local_paths_or_links() {
        assert_eq!(new_address("/about"), Some("/about".into()));
        assert_eq!(
            new_address("/hakkımızda"),
            Some("/hakk%C4%B1m%C4%B1zda".into())
        );
        assert_eq!(
            new_address("https://example.com/a page"),
            Some("https://example.com/a%20page".into())
        );
        for refused in ["", "about", "//evil.example", "javascript:alert(1)"] {
            assert_eq!(new_address(refused), None, "{refused}");
        }
    }
}
