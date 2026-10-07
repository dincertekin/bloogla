//! Putting themes in `themes/`: the bundled ones on start, uploaded .zip
//! files from Admin → Themes, and deleting uploaded themes (a copy is kept in
//! `data/theme-backups/`).

use super::{is_bundled, is_valid_id, load_theme, BundledThemes, THEMES_DIR};

use std::collections::HashSet;
use std::fs;
use std::io::{Cursor, Read};
use std::path::{Path, PathBuf};

/// Where replaced and deleted themes are kept, in case they're wanted back.
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

/// The largest theme .zip that can be uploaded.
pub const MAX_ZIP_BYTES: usize = 20 * 1024 * 1024;
/// Limits on what's inside, so a small .zip can't fill the disk.
const MAX_FILES: usize = 2_000;
const MAX_UNPACKED_BYTES: u64 = 100 * 1024 * 1024;

/// File types a theme may contain: templates, styles, scripts, data, images
/// and fonts. Anything else (programs, archives...) is refused.
const ALLOWED_EXTENSIONS: &[&str] = &[
    "html", "toml", "css", "js", "mjs", "json", "map", "txt", "md", "svg", "png", "jpg", "jpeg",
    "gif", "webp", "avif", "ico", "woff", "woff2", "ttf", "otf",
];

/// Install a theme from an uploaded .zip and return its folder name.
///
/// The .zip holds the theme's files, either at the top or inside one folder
/// (as GitHub downloads are). The folder name (or else the file name) becomes
/// the theme's id. The theme is unpacked into a hidden folder and checked
/// first; only a working theme replaces an installed one with the same id,
/// which is then kept in `data/theme-backups/`.
///
/// Errors are messages for the person uploading.
pub fn install_zip(data: &[u8], file_name: &str) -> Result<String, String> {
    install_zip_at(
        Path::new(THEMES_DIR),
        Path::new(BACKUP_DIR),
        data,
        file_name,
    )
}

fn install_zip_at(
    themes_dir: &Path,
    backup_dir: &Path,
    data: &[u8],
    file_name: &str,
) -> Result<String, String> {
    let mut archive = zip::ZipArchive::new(Cursor::new(data))
        .map_err(|_| "This file isn't a .zip archive.".to_string())?;

    // Every file in the .zip, checked before anything is written.
    let mut files: Vec<(PathBuf, usize)> = Vec::new();
    let mut unpacked: u64 = 0;
    for index in 0..archive.len() {
        let entry = archive
            .by_index(index)
            .map_err(|e| format!("Can't read the .zip: {e}"))?;
        // `enclosed_name` refuses paths like `../../etc/passwd`.
        let Some(path) = entry.enclosed_name() else {
            return Err(format!("Unsafe file path in the .zip: {}", entry.name()));
        };
        let hidden = path.components().any(|part| {
            let part = part.as_os_str().to_string_lossy();
            part.starts_with('.') || part == "__MACOSX"
        });
        if entry.is_dir() || hidden {
            continue;
        }
        let extension = path
            .extension()
            .map(|e| e.to_string_lossy().to_lowercase())
            .unwrap_or_default();
        if !ALLOWED_EXTENSIONS.contains(&extension.as_str()) {
            return Err(format!(
                "{} can't be in a theme. Themes may contain templates, styles, scripts, images and fonts.",
                path.display()
            ));
        }
        unpacked += entry.size();
        if files.len() >= MAX_FILES || unpacked > MAX_UNPACKED_BYTES {
            return Err("This .zip is too big when unpacked.".into());
        }
        files.push((path, index));
    }

    // theme.toml at the top, or inside a single folder.
    let has = |path: &str| files.iter().any(|(p, _)| p == Path::new(path));
    let folder = files
        .first()
        .and_then(|(p, _)| p.components().next())
        .map(|c| c.as_os_str().to_string_lossy().to_string());
    let prefix = if has("theme.toml") {
        None
    } else if let Some(folder) = folder
        .filter(|f| has(&format!("{f}/theme.toml")) && files.iter().all(|(p, _)| p.starts_with(f)))
    {
        Some(folder)
    } else {
        return Err("This .zip has no theme.toml, so it isn't a Bloogla theme.".into());
    };

    // The id: folder or file name, without GitHub's "-main" ending.
    let name = prefix
        .clone()
        .unwrap_or_else(|| file_name.trim_end_matches(".zip").to_string());
    let name = name
        .strip_suffix("-main")
        .or_else(|| name.strip_suffix("-master"))
        .unwrap_or(&name);
    let id = crate::content::text::slugify(name);
    if !is_valid_id(&id) {
        return Err(
            "Name the .zip or its folder with Latin letters and numbers, e.g. my-theme.zip.".into(),
        );
    }
    if is_bundled(&id) {
        return Err(format!(
            "\"{id}\" is the name of a theme that comes with Bloogla. Rename the .zip or its folder."
        ));
    }

    // Unpack into a hidden folder next to the themes, then check it.
    let staging = themes_dir.join(format!(".upload-{}", crate::app::security::random_hex(8)));
    let result =
        unpack(&mut archive, &files, prefix.as_deref(), &staging).and_then(|()| match load_theme(
            &id, &staging,
        )
        .info
        .error
        {
            Some(error) => Err(format!("This theme can't be used: {error}")),
            None => Ok(()),
        });
    if let Err(error) = result {
        let _ = fs::remove_dir_all(&staging);
        return Err(error);
    }

    let target = themes_dir.join(&id);
    let installed =
        move_to_backups(themes_dir, backup_dir, &id).and_then(|()| fs::rename(&staging, &target));
    if let Err(e) = installed {
        let _ = fs::remove_dir_all(&staging);
        tracing::error!("Failed to install theme {id}: {e}");
        return Err("Couldn't save the theme. Please try again.".into());
    }
    Ok(id)
}

/// Write the listed .zip files into `dir`, without the `prefix` folder.
fn unpack(
    archive: &mut zip::ZipArchive<Cursor<&[u8]>>,
    files: &[(PathBuf, usize)],
    prefix: Option<&str>,
    dir: &Path,
) -> Result<(), String> {
    let failed = |e: std::io::Error| {
        tracing::error!("Failed to unpack theme: {e}");
        "Couldn't save the theme. Please try again.".to_string()
    };
    let mut written: u64 = 0;
    for (path, index) in files {
        let relative = prefix.map_or(path.as_path(), |p| path.strip_prefix(p).unwrap_or(path));
        let target = dir.join(relative);
        if let Some(parent) = target.parent() {
            fs::create_dir_all(parent).map_err(failed)?;
        }
        let entry = archive
            .by_index(*index)
            .map_err(|e| format!("Can't read the .zip: {e}"))?;
        // Sizes in a .zip can lie; stop at the limit while unpacking too.
        let mut limited = entry.take(MAX_UNPACKED_BYTES - written + 1);
        let mut out = fs::File::create(&target).map_err(failed)?;
        written += std::io::copy(&mut limited, &mut out).map_err(failed)?;
        if written > MAX_UNPACKED_BYTES {
            return Err("This .zip is too big when unpacked.".into());
        }
    }
    Ok(())
}

/// Move an installed theme to `data/theme-backups/<id>-<version>` (replacing
/// an older backup of the same version). Does nothing if it isn't installed.
fn move_to_backups(themes_dir: &Path, backup_dir: &Path, id: &str) -> std::io::Result<()> {
    let installed = themes_dir.join(id);
    if !installed.exists() {
        return Ok(());
    }
    let version = fs::read_to_string(installed.join("theme.toml"))
        .ok()
        .and_then(|toml| theme_version(&toml))
        .unwrap_or_else(|| "unknown".into());
    let backup = backup_dir.join(format!("{id}-{version}"));
    fs::create_dir_all(backup_dir)?;
    if backup.exists() {
        fs::remove_dir_all(&backup)?;
    }
    fs::rename(installed, backup)
}

/// Delete an uploaded theme (a copy is kept in `data/theme-backups/`).
/// Themes that come with Bloogla can't be deleted.
pub fn delete(id: &str) -> Result<(), &'static str> {
    if !is_valid_id(id) || !Path::new(THEMES_DIR).join(id).is_dir() {
        return Err("This theme isn't installed.");
    }
    if is_bundled(id) {
        return Err("Themes that come with Bloogla can't be deleted.");
    }
    move_to_backups(Path::new(THEMES_DIR), Path::new(BACKUP_DIR), id).map_err(|e| {
        tracing::error!("Failed to delete theme {id}: {e}");
        "Couldn't delete the theme. Please try again."
    })
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
    use crate::services::themes::REQUIRED_TEMPLATES;
    use std::io::Cursor;

    /// A .zip with these `(path, contents)` files.
    fn zip_of(files: &[(&str, &str)]) -> Vec<u8> {
        use std::io::Write;
        let mut writer = zip::ZipWriter::new(Cursor::new(Vec::new()));
        for (path, contents) in files {
            writer
                .start_file(*path, zip::write::SimpleFileOptions::default())
                .unwrap();
            writer.write_all(contents.as_bytes()).unwrap();
        }
        writer.finish().unwrap().into_inner()
    }

    /// A working theme's files, inside `folder`.
    fn theme_files(folder: &str, version: &str) -> Vec<(String, String)> {
        let toml = format!(
            "name = \"Shop\"\nversion = \"{version}\"\nauthor = \"A\"\ndescription = \"D\"\n"
        );
        let mut files = vec![(format!("{folder}theme.toml"), toml)];
        for name in REQUIRED_TEMPLATES {
            files.push((format!("{folder}{name}"), "<p>{{ blog_name }}</p>".into()));
        }
        files.push((format!("{folder}static/style.css"), "p {}".into()));
        files
    }

    fn install(dir: &Path, files: &[(String, String)], file_name: &str) -> Result<String, String> {
        let files: Vec<(&str, &str)> = files
            .iter()
            .map(|(p, c)| (p.as_str(), c.as_str()))
            .collect();
        install_zip_at(
            &dir.join("themes"),
            &dir.join("backups"),
            &zip_of(&files),
            file_name,
        )
    }

    fn temp_dir() -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "bloogla-test-{}",
            crate::app::security::random_hex(8)
        ));
        fs::create_dir_all(dir.join("themes")).unwrap();
        dir
    }

    #[test]
    fn installs_a_theme_from_a_github_style_zip_and_keeps_the_old_one() {
        let dir = temp_dir();
        let id = install(&dir, &theme_files("shop-main/", "1.0.0"), "x.zip").unwrap();
        assert_eq!(id, "shop");
        assert!(dir.join("themes/shop/templates/index.html").exists());
        assert!(dir.join("themes/shop/static/style.css").exists());

        // Uploading it again replaces it and keeps the previous version.
        let id = install(&dir, &theme_files("", "1.1.0"), "shop.zip").unwrap();
        assert_eq!(id, "shop");
        assert!(dir.join("backups/shop-1.0.0/theme.toml").exists());
        assert!(fs::read_to_string(dir.join("themes/shop/theme.toml"))
            .unwrap()
            .contains("1.1.0"));

        // No unfinished uploads are left behind.
        let hidden = fs::read_dir(dir.join("themes"))
            .unwrap()
            .flatten()
            .any(|e| e.file_name().to_string_lossy().starts_with('.'));
        assert!(!hidden);
        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn refuses_unsafe_or_broken_themes() {
        let dir = temp_dir();
        let with = |extra: (&str, &str)| {
            let mut files = theme_files("", "1.0.0");
            files.push((extra.0.into(), extra.1.into()));
            files
        };

        // Paths that leave the theme folder, and files that aren't theme files.
        assert!(install(&dir, &with(("../evil.html", "x")), "a.zip").is_err());
        assert!(install(&dir, &with(("shell.php", "x")), "a.zip").is_err());
        // Template errors are found before anything is installed.
        let broken = install(&dir, &with(("templates/page.html", "{% if %}")), "a.zip");
        assert!(broken.unwrap_err().contains("can't be used"));
        // Not a theme, not a .zip, and a bundled theme's name.
        assert!(install(&dir, &[("readme.txt".into(), "hi".into())], "a.zip").is_err());
        assert!(install_zip_at(&dir.join("themes"), &dir, b"not a zip", "a.zip").is_err());
        assert!(install(&dir, &theme_files("", "9.0.0"), "default.zip").is_err());

        assert_eq!(fs::read_dir(dir.join("themes")).unwrap().count(), 0);
        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn compares_versions_numerically() {
        assert!(is_newer("1.10.0", "1.9.2"));
        assert!(is_newer("1.1.0", "1.0.0"));
        assert!(!is_newer("1.0.0", "1.1.0"));
        assert!(!is_newer("1.0.0", "1.0.0"));
    }
}
