//! Putting the bundled themes in `themes/` on start, and upgrading them when
//! a new Bloogla version ships newer ones.

use super::{BundledThemes, THEMES_DIR};

use std::collections::HashSet;
use std::fs;
use std::path::{Path, PathBuf};

/// Where replaced themes are kept, in case they're wanted back.
const BACKUP_DIR: &str = "data/theme-backups";

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
                let backup = PathBuf::from(BACKUP_DIR).join(format!("{name}-{old}"));
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
    fn compares_versions_numerically() {
        assert!(is_newer("1.10.0", "1.9.2"));
        assert!(is_newer("1.1.0", "1.0.0"));
        assert!(!is_newer("1.0.0", "1.1.0"));
        assert!(!is_newer("1.0.0", "1.0.0"));
    }
}
