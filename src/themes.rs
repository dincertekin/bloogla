use serde::Deserialize;
use std::fs;
use std::path::PathBuf;

#[derive(Debug, Clone, Deserialize)]
pub struct ThemeMeta {
    pub name: String,
    pub version: String,
    pub author: String,
    pub description: String,
    pub preview_image: Option<String>,
}

#[derive(Debug, Clone)]
pub struct ThemeInfo {
    pub id: String, // Folder name, e.g., "default"
    pub meta: ThemeMeta,
}

pub fn discover_themes() -> Vec<ThemeInfo> {
    let mut themes = Vec::new();
    let themes_dir = PathBuf::from("./themes");

    if let Ok(entries) = fs::read_dir(themes_dir) {
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() {
                let id = path.file_name().unwrap().to_string_lossy().to_string();
                let manifest_path = path.join("theme.toml");

                if manifest_path.exists() {
                    if let Ok(content) = fs::read_to_string(manifest_path) {
                        if let Ok(meta) = toml::from_str::<ThemeMeta>(&content) {
                            themes.push(ThemeInfo { id, meta });
                        }
                    }
                }
            }
        }
    }
    themes
}
