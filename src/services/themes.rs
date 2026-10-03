//! Themes: the folders in `themes/` that decide how the public site looks.
//!
//! A theme is a folder with a `theme.toml` and Tera templates (see the README).
//! The bundled `default` theme is compiled into the binary and written to
//! `themes/default` on first start.

use crate::app::state::AppState;

use axum::http::{header, HeaderValue, StatusCode};
use axum::response::{Html, IntoResponse, Response};
use rust_embed::RustEmbed;
use serde::Deserialize;
use std::collections::{BTreeMap, HashMap, HashSet};
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::RwLock;
use tera::Tera;

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

/// `theme.toml` of every installed theme, read when templates are loaded.
static INSTALLED: RwLock<Option<HashMap<String, ThemeMeta>>> = RwLock::new(None);

fn remember_installed() {
    let themes = discover().into_iter().map(|t| (t.id, t.meta)).collect();
    if let Ok(mut installed) = INSTALLED.write() {
        *installed = Some(themes);
    }
}

fn installed_meta(theme: &str) -> Option<ThemeMeta> {
    INSTALLED.read().ok()?.as_ref()?.get(theme).cloned()
}

#[derive(Debug, Clone)]
pub struct ThemeInfo {
    /// Folder name, e.g. `default`.
    pub id: String,
    pub meta: ThemeMeta,
}

/// Installed themes that have a valid `theme.toml`.
pub fn discover() -> Vec<ThemeInfo> {
    let Ok(entries) = fs::read_dir(THEMES_DIR) else {
        return Vec::new();
    };
    let mut themes: Vec<ThemeInfo> = entries
        .flatten()
        .filter_map(|entry| {
            let id = entry.file_name().to_str()?.to_string();
            let manifest = fs::read_to_string(entry.path().join("theme.toml")).ok()?;
            let meta = toml::from_str(&manifest).ok()?;
            Some(ThemeInfo { id, meta })
        })
        .collect();
    themes.sort_by(|a, b| a.id.cmp(&b.id));
    themes
}

/// Parse the templates of every installed theme.
pub fn load_templates() -> Result<Tera, String> {
    remember_installed();
    Tera::new(&format!("{THEMES_DIR}/**/*.html")).map_err(|e| format!("Theme parsing error: {e:?}"))
}

/// Render the first template of the active theme that exists among `candidates`
/// (paths inside the theme folder, e.g. `templates/page.html`).
///
/// Adds `asset_version` (the theme's version, for cache-busting asset URLs)
/// to the context, and sends the theme's Content Security Policy.
///
/// In debug builds templates are reloaded on every request, so theme edits
/// show up without restarting.
pub async fn render(
    state: &AppState,
    candidates: &[&str],
    mut context: tera::Context,
    status: StatusCode,
) -> Response {
    let active_theme = crate::db::settings::load(&state.pool)
        .await
        .active_theme
        .clone();

    if cfg!(debug_assertions) {
        if let Ok(mut tera) = state.tera.write() {
            if let Err(e) = tera.full_reload() {
                tracing::error!("Theme reload failed: {e:?}");
            }
            remember_installed();
        }
    }

    let Ok(tera) = state.tera.read() else {
        return theme_error("template engine unavailable");
    };
    // A removed or broken theme falls back to the bundled one.
    let has_template = |name: &str| tera.get_template_names().any(|t| t == name);
    let theme = if tera
        .get_template_names()
        .any(|t| t.starts_with(&format!("{active_theme}/")))
    {
        active_theme.as_str()
    } else {
        "default"
    };
    let template = candidates
        .iter()
        .map(|path| format!("{theme}/{path}"))
        .find(|name| has_template(name));

    let Some(template) = template else {
        return if status == StatusCode::NOT_FOUND {
            (status, Html("<h1>404 Not Found</h1>")).into_response()
        } else {
            theme_error(&format!("theme '{theme}' is missing {candidates:?}"))
        };
    };

    let meta = installed_meta(theme);
    let version = meta.as_ref().map_or("0", |m| m.version.as_str());
    context.insert("asset_version", version);
    match tera.render(&template, &context) {
        Ok(html) => {
            let mut response = (status, Html(html)).into_response();
            if let Ok(policy) = HeaderValue::from_str(&content_security_policy(meta.as_ref())) {
                response
                    .headers_mut()
                    .insert(header::CONTENT_SECURITY_POLICY, policy);
            }
            response
        }
        Err(err) => theme_error(&format!("{template}: {err:?}")),
    }
}

fn theme_error(detail: &str) -> Response {
    tracing::error!("Theme rendering error: {detail}");
    (
        StatusCode::INTERNAL_SERVER_ERROR,
        Html("<h1>500 - Theme Error</h1><p>Failed to render theme template.</p>"),
    )
        .into_response()
}

/// Install bundled themes into `./themes/<name>`, and upgrade them when the
/// binary ships a newer version (per `theme.toml`).
///
/// Before an upgrade the installed copy is moved to
/// `data/theme-backups/<name>-<version>`, so local edits are never lost.
/// Returns a line per theme installed or upgraded.
pub fn install_bundled() -> std::io::Result<Vec<String>> {
    let names: HashSet<String> = BundledThemes::iter()
        .filter_map(|path| path.split('/').next().map(str::to_string))
        .collect();

    let mut report = Vec::new();
    for name in names {
        let target = Path::new(THEMES_DIR).join(&name);
        let bundled = BundledThemes::get(&format!("{name}/theme.toml"))
            .and_then(|f| theme_version(&String::from_utf8_lossy(&f.data)));
        let installed = fs::read_to_string(target.join("theme.toml"))
            .ok()
            .and_then(|toml| theme_version(&toml));

        match (&installed, &bundled) {
            (None, _) if !target.exists() => report.push(format!("Installed theme: {name}")),
            (Some(old), Some(new)) if is_newer(new, old) => {
                let backup = PathBuf::from("data/theme-backups").join(format!("{name}-{old}"));
                if let Some(parent) = backup.parent() {
                    fs::create_dir_all(parent)?;
                }
                if backup.exists() {
                    fs::remove_dir_all(&target)?;
                } else {
                    fs::rename(&target, &backup)?;
                }
                report.push(format!(
                    "Updated theme {name} {old} -> {new} (previous copy in {})",
                    backup.display()
                ));
            }
            _ => continue,
        }

        for path in BundledThemes::iter().filter(|p| p.starts_with(&format!("{name}/"))) {
            let file_target = Path::new(THEMES_DIR).join(path.as_ref());
            if let Some(parent) = file_target.parent() {
                fs::create_dir_all(parent)?;
            }
            if let Some(file) = BundledThemes::get(&path) {
                fs::write(file_target, file.data)?;
            }
        }
    }

    report.sort();
    Ok(report)
}

/// True when dotted version `a` is greater than `b` ("1.10.0" > "1.9.2").
fn is_newer(a: &str, b: &str) -> bool {
    let parse = |v: &str| -> Vec<u64> { v.split('.').map(|p| p.parse().unwrap_or(0)).collect() };
    parse(a) > parse(b)
}

/// The `version = "..."` value of a theme.toml.
fn theme_version(toml_text: &str) -> Option<String> {
    toml::from_str::<toml::Table>(toml_text)
        .ok()?
        .get("version")?
        .as_str()
        .map(str::to_string)
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
    fn compares_versions_numerically() {
        assert!(is_newer("1.10.0", "1.9.2"));
        assert!(is_newer("1.1.0", "1.0.0"));
        assert!(!is_newer("1.0.0", "1.1.0"));
        assert!(!is_newer("1.0.0", "1.0.0"));
    }
}
