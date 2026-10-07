//! Themes: the folders in `themes/` that decide how the public site looks.
//!
//! A theme is a folder with a `theme.toml` and Tera templates (see the README).
//! The bundled `default` theme is compiled into the binary and written to
//! `themes/default` on first start. Admins can add more by uploading a .zip
//! (see [`install_zip`]).
//!
//! Every theme's templates are loaded separately, so a broken theme is only
//! marked as broken (with the reason, shown in Admin → Themes) and the site
//! keeps working with the default theme.
//!
//! - `install`: bundled themes, .zip uploads and deleting
//! - `options`: the settings a theme offers (colors, texts...)
//! - `starter`: the sample content a new site begins with

use crate::app::state::AppState;

use axum::http::{header, HeaderValue, StatusCode};
use axum::response::{Html, IntoResponse, Response};
use rust_embed::RustEmbed;
use serde::Deserialize;
use std::collections::BTreeMap;
use std::fs;
use std::path::Path;
use tera::Tera;

mod install;
pub mod options;
pub mod starter;
pub use install::{delete, install_bundled, install_zip, MAX_ZIP_BYTES};
use options::ThemeOption;

/// Where installed themes live, one folder each.
const THEMES_DIR: &str = "themes";

/// Themes shipped with Bloogla.
#[derive(RustEmbed)]
#[folder = "themes/"]
#[exclude = "*.DS_Store"]
struct BundledThemes;

/// The `theme.toml` of a theme.
#[derive(Debug, Clone, Deserialize)]
pub struct ThemeMeta {
    pub name: String,
    pub version: String,
    pub author: String,
    pub description: String,
    pub preview_image: Option<String>,
    /// Extra sources the theme loads from, added to the page's Content
    /// Security Policy, e.g. `font-src = ["https://fonts.gstatic.com"]`
    /// under a `[csp]` table.
    #[serde(default)]
    pub csp: BTreeMap<String, Vec<String>>,
    /// Settings the theme offers, as `[[options]]` (see `options.rs`).
    #[serde(default)]
    pub options: Vec<ThemeOption>,
}

/// Content Security Policy for theme pages: scripts, styles and fonts from the
/// site itself; images, audio and video from any HTTPS site (posts link to
/// them); YouTube and Vimeo embeds. Themes can add sources in `theme.toml`.
const PUBLIC_CSP: &[(&str, &str)] = &[
    ("default-src", "'self'"),
    ("script-src", "'self'"),
    ("style-src", "'self' 'unsafe-inline'"),
    ("img-src", "'self' https: data:"),
    ("media-src", "'self' https:"),
    (
        "frame-src",
        "https://www.youtube-nocookie.com https://player.vimeo.com",
    ),
    ("connect-src", "'self'"),
    ("font-src", "'self'"),
    ("object-src", "'none'"),
    ("base-uri", "'self'"),
    ("form-action", "'self'"),
    ("frame-ancestors", "'none'"),
];

/// The page policy for a theme: [`PUBLIC_CSP`] plus the theme's own sources.
fn content_security_policy(meta: Option<&ThemeMeta>) -> String {
    let mut policy: Vec<(String, String)> = PUBLIC_CSP
        .iter()
        .map(|(d, s)| (d.to_string(), s.to_string()))
        .collect();
    for (directive, sources) in meta.map(|m| &m.csp).into_iter().flatten() {
        // Only plain names and sources; nothing that could end the header early.
        let is_safe =
            |s: &str| !s.is_empty() && !s.contains([';', ',', '\n', '\r']) && !s.contains(' ');
        if !is_safe(directive) {
            continue;
        }
        let extra: Vec<&str> = sources
            .iter()
            .map(String::as_str)
            .filter(|s| is_safe(s))
            .collect();
        match policy.iter_mut().find(|(d, _)| d == directive) {
            Some((_, existing)) => existing.extend(extra.iter().map(|s| format!(" {s}"))),
            None => policy.push((directive.clone(), extra.join(" "))),
        }
    }
    policy
        .iter()
        .map(|(d, s)| format!("{d} {s}"))
        .collect::<Vec<_>>()
        .join("; ")
}

/// Templates every theme must have.
const REQUIRED_TEMPLATES: &[&str] = &[
    "templates/index.html",
    "templates/post.html",
    "templates/tag.html",
    "templates/message.html",
];

#[derive(Debug, Clone)]
pub struct ThemeInfo {
    /// Folder name, e.g. `default`.
    pub id: String,
    pub meta: ThemeMeta,
    /// Why the theme can't be used, if it can't.
    pub error: Option<String>,
}

impl ThemeInfo {
    /// True for themes that come with Bloogla (they can't be deleted).
    pub fn is_bundled(&self) -> bool {
        is_bundled(&self.id)
    }

    /// The preview picture's address. `preview_image` in `theme.toml` may be a
    /// path inside the theme (`static/preview.png`) or a full URL.
    pub fn preview_url(&self) -> Option<String> {
        let image = self.meta.preview_image.as_deref()?.trim();
        if image.is_empty() {
            None
        } else if image.starts_with('/') || image.starts_with("https://") {
            Some(image.to_string())
        } else {
            Some(format!("/theme-assets/{}/{image}", self.id))
        }
    }
}

fn is_bundled(id: &str) -> bool {
    BundledThemes::get(&format!("{id}/theme.toml")).is_some()
}

/// A theme ready to render pages with.
pub struct UsableTheme<'a> {
    pub id: &'a str,
    pub meta: &'a ThemeMeta,
    pub tera: &'a Tera,
}

struct LoadedTheme {
    info: ThemeInfo,
    /// `None` when the theme is broken.
    tera: Option<Tera>,
}

/// Every installed theme with its templates, kept in `AppState`.
#[derive(Default)]
pub struct Themes {
    themes: BTreeMap<String, LoadedTheme>,
}

impl Themes {
    /// Read every theme folder in `themes/`.
    pub fn load() -> Self {
        let mut themes = Themes::default();
        if let Ok(entries) = fs::read_dir(THEMES_DIR) {
            for entry in entries.flatten() {
                let Some(id) = entry.file_name().to_str().map(str::to_string) else {
                    continue;
                };
                // Hidden folders (unfinished uploads) aren't themes.
                if !id.starts_with('.') {
                    themes.reload(&id);
                }
            }
        }
        for theme in themes.themes.values() {
            if let Some(error) = &theme.info.error {
                tracing::warn!("Theme '{}' can't be used: {error}", theme.info.id);
            }
        }
        themes
    }

    /// Read one theme again (after it was uploaded, changed or deleted).
    pub fn reload(&mut self, id: &str) {
        let dir = Path::new(THEMES_DIR).join(id);
        if dir.is_dir() {
            self.themes.insert(id.to_string(), load_theme(id, &dir));
        } else {
            self.themes.remove(id);
        }
    }

    /// Every installed theme, the bundled ones first.
    pub fn list(&self) -> Vec<ThemeInfo> {
        let mut list: Vec<ThemeInfo> = self.themes.values().map(|t| t.info.clone()).collect();
        list.sort_by_key(|t| (!t.is_bundled(), t.meta.name.to_lowercase()));
        list
    }

    /// The theme `id` if it's installed and works.
    pub fn usable(&self, id: &str) -> Option<UsableTheme<'_>> {
        let theme = self.themes.get(id)?;
        Some(UsableTheme {
            id: &theme.info.id,
            meta: &theme.info.meta,
            tera: theme.tera.as_ref()?,
        })
    }

    /// The theme to show visitors: the chosen one, or the default theme if
    /// the chosen one was removed or is broken.
    pub fn pick(&self, chosen: &str) -> Option<UsableTheme<'_>> {
        self.usable(chosen).or_else(|| self.usable("default"))
    }
}

/// Folder names that are safe in URLs and file paths: `my-theme_2`.
fn is_valid_id(id: &str) -> bool {
    !id.is_empty()
        && id.len() <= 64
        && id
            .chars()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-' || c == '_')
}

/// Read a theme's `theme.toml` and templates from `dir`.
fn load_theme(id: &str, dir: &Path) -> LoadedTheme {
    let broken = |meta: ThemeMeta, error: String| LoadedTheme {
        info: ThemeInfo {
            id: id.to_string(),
            meta,
            error: Some(error),
        },
        tera: None,
    };
    let unnamed = || ThemeMeta {
        name: id.to_string(),
        version: "?".into(),
        author: String::new(),
        description: String::new(),
        preview_image: None,
        csp: BTreeMap::new(),
        options: Vec::new(),
    };

    if !is_valid_id(id) {
        return broken(
            unnamed(),
            "The folder name may only use lowercase letters, numbers, - and _.".into(),
        );
    }
    let meta: ThemeMeta = match fs::read_to_string(dir.join("theme.toml")) {
        Err(_) => return broken(unnamed(), "theme.toml is missing.".into()),
        Ok(text) => match toml::from_str(&text) {
            Ok(meta) => meta,
            Err(e) => return broken(unnamed(), format!("theme.toml: {}", e.message())),
        },
    };
    if let Err(problem) = options::check(&meta.options) {
        return broken(meta, format!("theme.toml: {problem}"));
    }
    let pattern = format!("{}/**/*.html", dir.display());
    let tera = match Tera::new(&pattern) {
        Ok(tera) => tera,
        Err(e) => {
            // Paths inside the theme are enough: `templates/index.html`.
            let mut error = describe_error(&e);
            if let Ok(full) = dir.canonicalize() {
                error = error.replace(&format!("{}/", full.display()), "");
            }
            return broken(meta, error.trim_start_matches(['\n', '*', ' ']).to_string());
        }
    };
    if let Some(missing) = REQUIRED_TEMPLATES
        .iter()
        .find(|name| !tera.get_template_names().any(|t| t == **name))
    {
        return broken(meta, format!("{missing} is missing."));
    }

    LoadedTheme {
        info: ThemeInfo {
            id: id.to_string(),
            meta,
            error: None,
        },
        tera: Some(tera),
    }
}

/// A template error with its causes ("Failed to parse 'x': expected ...").
fn describe_error(error: &tera::Error) -> String {
    let mut text = error.to_string();
    let mut source = std::error::Error::source(error);
    while let Some(cause) = source {
        text.push_str(": ");
        text.push_str(&cause.to_string());
        source = cause.source();
    }
    text
}

// ---- Previewing a theme ----
//
// An admin can look at the site in another theme before switching (Admin →
// Themes → Preview). The theme name is kept in their session, and the
// `theme_preview` middleware makes it available while their requests run.

/// Session key holding the theme an admin is previewing.
pub const PREVIEW_SESSION_KEY: &str = "preview_theme";

tokio::task_local! {
    static PREVIEW: Option<String>;
}

/// Run `request` with `theme` as the theme being previewed.
pub async fn with_preview<F: std::future::Future>(theme: Option<String>, request: F) -> F::Output {
    PREVIEW.scope(theme, request).await
}

/// The theme previewed during this request, if any.
fn previewing() -> Option<String> {
    PREVIEW.try_with(Clone::clone).ok().flatten()
}

/// The theme to use for this request: the one being previewed, or the
/// site's active theme.
pub fn chosen_theme(active: &str) -> String {
    previewing().unwrap_or_else(|| active.to_string())
}

/// Render the first template of the active theme that exists among `candidates`
/// (paths inside the theme folder, e.g. `templates/page.html`).
///
/// Adds to the context: `asset_version` (the theme's version, for
/// cache-busting asset URLs), `theme_url` (where the theme's files are served,
/// e.g. `/theme-assets/default`) and `theme_options`. Sends the theme's
/// Content Security Policy.
///
/// In debug builds the active theme is reloaded on every request, so theme
/// edits show up without restarting.
pub async fn render(
    state: &AppState,
    candidates: &[&str],
    mut context: tera::Context,
    status: StatusCode,
) -> Response {
    let site = crate::db::settings::load(&state.pool).await;
    let chosen = chosen_theme(&site.active_theme);

    if cfg!(debug_assertions) {
        if let Ok(mut themes) = state.themes.write() {
            themes.reload(&chosen);
        }
    }

    let Ok(themes) = state.themes.read() else {
        return theme_error("themes unavailable");
    };
    let Some(theme) = themes.pick(&chosen) else {
        return theme_error("no working theme is installed");
    };
    let has_template = |name: &str| theme.tera.get_template_names().any(|t| t == name);
    let Some(template) = candidates.iter().find(|name| has_template(name)) else {
        return if status == StatusCode::NOT_FOUND {
            (status, Html("<h1>404 Not Found</h1>")).into_response()
        } else {
            theme_error(&format!("theme '{}' is missing {candidates:?}", theme.id))
        };
    };

    context.insert("asset_version", &theme.meta.version);
    context.insert("theme_url", &format!("/theme-assets/{}", theme.id));
    context.insert(
        "theme_options",
        &options::for_templates(theme.id, &theme.meta.options, &site.theme_options),
    );
    let html = match theme.tera.render(template, &context) {
        Ok(html) => html,
        Err(err) => {
            return theme_error(&format!(
                "{}/{template}: {}",
                theme.id,
                describe_error(&err)
            ))
        }
    };

    // The preview bar, only for an admin previewing a theme that isn't active.
    let preview = previewing().as_deref() == Some(theme.id) && theme.id != site.active_theme;
    let html = if preview {
        with_preview_bar(html, &theme, site.language)
    } else {
        html
    };
    let mut response = (status, Html(html)).into_response();
    let headers = response.headers_mut();
    if let Ok(policy) = HeaderValue::from_str(&content_security_policy(Some(theme.meta))) {
        headers.insert(header::CONTENT_SECURITY_POLICY, policy);
    }
    if preview {
        // Only for this admin, and never kept by the browser.
        headers.insert(
            header::CACHE_CONTROL,
            HeaderValue::from_static("private, no-store"),
        );
    }
    response
}

/// Put a bar at the top of a previewed page: what's shown, with buttons to
/// switch to the theme or stop the preview.
fn with_preview_bar(html: String, theme: &UsableTheme<'_>, lang: crate::i18n::Lang) -> String {
    use crate::content::text::escape_html;

    let button = "margin:0;padding:.35rem .8rem;border:1px solid #4b5563;border-radius:6px;\
                  background:#1f2937;color:inherit;font:inherit;cursor:pointer";
    let bar = format!(
        r#"<div style="position:sticky;top:0;z-index:2147483647;display:flex;flex-wrap:wrap;gap:.5rem 1rem;align-items:center;justify-content:center;padding:.6rem 1rem;background:#111827;color:#f9fafb;font:14px/1.4 system-ui,sans-serif">
<span>{text}</span>
<form method="post" action="/admin/themes/activate" style="margin:0"><input type="hidden" name="theme" value="{id}"><button style="{button};background:#f9fafb;color:#111827">{use_it}</button></form>
<form method="post" action="/admin/themes/preview/stop" style="margin:0"><button style="{button}">{stop}</button></form>
</div>"#,
        text = lang.tv(
            "Previewing {name}. Only you can see this.",
            format!("<strong>{}</strong>", escape_html(&theme.meta.name))
        ),
        id = escape_html(theme.id),
        use_it = lang.t("Use this theme"),
        stop = lang.t("Stop preview"),
    );
    // Right after the opening <body> tag, or at the very top.
    let at = html
        .find("<body")
        .and_then(|start| html[start..].find('>').map(|end| start + end + 1))
        .unwrap_or(0);
    let mut out = html;
    out.insert_str(at, &bar);
    out
}

fn theme_error(detail: &str) -> Response {
    tracing::error!("Theme rendering error: {detail}");
    (
        StatusCode::INTERNAL_SERVER_ERROR,
        Html("<h1>500 - Theme Error</h1><p>Failed to render theme template.</p>"),
    )
        .into_response()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn themes_can_add_policy_sources_but_not_break_the_header() {
        let mut meta: ThemeMeta = toml::from_str(
            r#"
            name = "T"
            version = "1"
            author = "A"
            description = "D"
            [csp]
            font-src = ["https://fonts.gstatic.com", "x; script-src *"]
            worker-src = ["'self'"]
            "#,
        )
        .unwrap();
        let policy = content_security_policy(Some(&meta));
        assert!(policy.contains("font-src 'self' https://fonts.gstatic.com;"));
        assert!(policy.contains("worker-src 'self'"));
        assert!(!policy.contains("script-src *"));
        meta.csp.clear();
        assert!(content_security_policy(Some(&meta))
            .starts_with("default-src 'self'; script-src 'self';"));
    }

    #[test]
    fn bundled_themes_load() {
        let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("themes");
        for entry in fs::read_dir(dir).unwrap().flatten() {
            let id = entry.file_name().to_string_lossy().to_string();
            if let Some(error) = load_theme(&id, &entry.path()).info.error {
                panic!("theme {id} doesn't load: {error}");
            }
        }
    }
}
