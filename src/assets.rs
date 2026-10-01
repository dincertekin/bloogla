//! Files compiled into the binary, so a deploy is a single executable.

use axum::extract::Path;
use axum::http::{header, HeaderMap, StatusCode};
use axum::response::{IntoResponse, Response};
use rust_embed::RustEmbed;
use std::collections::HashSet;
use std::fs;
use std::path::PathBuf;

/// Admin panel CSS and JS, served from memory at `/static`.
#[derive(RustEmbed)]
#[folder = "static/"]
struct StaticAssets;

/// Themes shipped with Bloogla, written to `./themes` on first start.
#[derive(RustEmbed)]
#[folder = "themes/"]
#[exclude = "*.DS_Store"]
struct BundledThemes;

/// GET /static/*path -> Embedded admin asset, with ETag revalidation.
pub async fn serve_static(Path(path): Path<String>, headers: HeaderMap) -> Response {
    let Some(file) = StaticAssets::get(&path) else {
        return StatusCode::NOT_FOUND.into_response();
    };

    let etag = format!(
        "\"{}\"",
        file.metadata
            .sha256_hash()
            .iter()
            .take(8)
            .map(|b| format!("{b:02x}"))
            .collect::<String>()
    );
    let cache_headers = [
        (header::ETAG, etag.clone()),
        // Not fingerprinted, so revalidate daily to pick up upgrades.
        (header::CACHE_CONTROL, "public, max-age=86400".to_string()),
    ];

    if headers
        .get(header::IF_NONE_MATCH)
        .is_some_and(|v| v.as_bytes() == etag.as_bytes())
    {
        return (StatusCode::NOT_MODIFIED, cache_headers).into_response();
    }

    (
        cache_headers,
        [(header::CONTENT_TYPE, file.metadata.mimetype().to_string())],
        file.data.into_owned(),
    )
        .into_response()
}

/// Install bundled themes into `./themes/<name>`, and upgrade them when the
/// binary ships a newer version (per `theme.toml`).
///
/// Before an upgrade the installed copy is moved to
/// `data/theme-backups/<name>-<version>`, so local edits are never lost.
/// Returns a line per theme installed or upgraded.
pub fn install_bundled_themes() -> std::io::Result<Vec<String>> {
    let names: HashSet<String> = BundledThemes::iter()
        .filter_map(|path| path.split('/').next().map(str::to_string))
        .collect();

    let mut report = Vec::new();
    for name in names {
        let target = PathBuf::from("themes").join(&name);
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
            let file_target = PathBuf::from("themes").join(path.as_ref());
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

#[cfg(test)]
mod tests {
    use super::is_newer;

    #[test]
    fn compares_versions_numerically() {
        assert!(is_newer("1.10.0", "1.9.2"));
        assert!(is_newer("1.1.0", "1.0.0"));
        assert!(!is_newer("1.0.0", "1.1.0"));
        assert!(!is_newer("1.0.0", "1.0.0"));
    }
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
