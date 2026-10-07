//! Starter content: the sample posts, pages and menu a new site begins with,
//! so it doesn't start empty. They come from the theme's `starter/` folder
//! and are added once, at setup.
//!
//! `starter/site.toml` sets the site description and menu:
//!
//! ```toml
//! description = "Design and photography by Ada."
//! menu = "Work | /\nAbout | /about"
//! ```
//!
//! Every other `starter/*.md` file is a post or page: Markdown with a TOML
//! header between `+++` lines. Files are added in name order, and the first
//! one is shown first (newest).
//!
//! ```text
//! +++
//! title = "About"
//! page = true                 # a page instead of a post
//! tags = ["Branding"]
//! cover = "/theme-assets/portfolio/static/starter/one.svg"
//! [fields]                    # custom fields, e.g. post.fields.client
//! client = "Northwind"
//! +++
//! The text, in Markdown.
//! ```

use super::THEMES_DIR;
use crate::db::{fields, posts, settings, tags};

use serde::Deserialize;
use sqlx::SqlitePool;
use std::collections::BTreeMap;
use std::fs;
use std::path::Path;

#[derive(Deserialize, Default)]
struct SiteFile {
    description: Option<String>,
    menu: Option<String>,
}

#[derive(Deserialize)]
struct Header {
    title: String,
    #[serde(default)]
    page: bool,
    #[serde(default)]
    tags: Vec<String>,
    cover: Option<String>,
    #[serde(default)]
    fields: BTreeMap<String, String>,
}

/// One starter post or page, read from its file.
struct Entry {
    header: Header,
    content: String,
}

/// Split a starter file into its header and Markdown.
fn parse(text: &str) -> Result<Entry, String> {
    let rest = text
        .trim_start()
        .strip_prefix("+++")
        .ok_or("starts without a +++ header")?;
    let (header, content) = rest.split_once("\n+++").ok_or("has no closing +++")?;
    let header: Header = toml::from_str(header).map_err(|e| e.message().to_string())?;
    Ok(Entry {
        header,
        content: content
            .trim_start_matches(['\r', '\n'])
            .trim_end()
            .to_string(),
    })
}

/// Add the starter content of `theme`, written by `author_id`. Returns how
/// many posts and pages were added. Problems are logged and skipped: a new
/// site works without its sample content.
pub async fn install(pool: &SqlitePool, theme: &str, author_id: i64) -> usize {
    let dir = Path::new(THEMES_DIR).join(theme).join("starter");
    let Ok(entries) = fs::read_dir(&dir) else {
        return 0;
    };
    let mut files: Vec<_> = entries.flatten().map(|e| e.path()).collect();
    files.sort();

    if let Ok(text) = fs::read_to_string(dir.join("site.toml")) {
        match toml::from_str::<SiteFile>(&text) {
            Ok(site) => {
                let mut values = Vec::new();
                if let Some(description) = site.description {
                    values.push(("blog_description", description));
                }
                if let Some(menu) = site.menu {
                    values.push(("nav_menu", menu));
                }
                if let Err(e) = settings::save(pool, &values).await {
                    tracing::error!("Starter settings of {theme} not saved: {e}");
                }
            }
            Err(e) => tracing::warn!("{theme}/starter/site.toml: {}", e.message()),
        }
    }

    let mut added = 0;
    let markdown = files
        .iter()
        .filter(|p| p.extension().is_some_and(|e| e == "md"));
    for (position, path) in markdown.enumerate() {
        let entry = match fs::read_to_string(path)
            .map_err(|e| e.to_string())
            .and_then(|t| parse(&t))
        {
            Ok(entry) => entry,
            Err(e) => {
                tracing::warn!("Starter file {} skipped: it {e}", path.display());
                continue;
            }
        };
        match add(pool, &entry, author_id, position).await {
            Ok(()) => added += 1,
            Err(e) => tracing::error!("Starter file {} not added: {e}", path.display()),
        }
    }
    added
}

/// Save one starter post or page. `position` 0 is the newest.
async fn add(
    pool: &SqlitePool,
    entry: &Entry,
    author_id: i64,
    position: usize,
) -> Result<(), sqlx::Error> {
    let header = &entry.header;
    let slug = posts::unique_slug(pool, &header.title, None).await?;
    // A minute apart, so lists show them in file order.
    let published = (chrono::Utc::now() - chrono::Duration::minutes(position as i64))
        .format("%Y-%m-%d %H:%M:%S")
        .to_string();
    let id = sqlx::query(
        "INSERT INTO posts (title, slug, content, cover_image, status, published_at, is_page, author_id)
         VALUES (?, ?, ?, ?, 'published', ?, ?, ?)",
    )
    .bind(&header.title)
    .bind(&slug)
    .bind(&entry.content)
    .bind(&header.cover)
    .bind(&published)
    .bind(header.page)
    .bind(author_id)
    .execute(pool)
    .await?
    .last_insert_rowid();

    let mut tag_ids = Vec::new();
    for name in &header.tags {
        let id = match tags::find_id(pool, name).await {
            Some(id) => id,
            None => tags::create(pool, name).await?,
        };
        tag_ids.push(id);
    }
    tags::set_for_post(pool, id, &tag_ids).await;
    fields::set_for_post(pool, id, &fields::clean(header.fields.clone())).await;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_the_header_and_text() {
        let entry = parse(
            "+++\ntitle = \"About\"\npage = true\ntags = [\"A\"]\n[fields]\nclient = \"N\"\n+++\n\nHello **there**.\n",
        )
        .unwrap();
        assert_eq!(entry.header.title, "About");
        assert!(entry.header.page);
        assert_eq!(entry.header.fields["client"], "N");
        assert_eq!(entry.content, "Hello **there**.");
        assert!(parse("no header").is_err());
    }

    #[test]
    fn bundled_starter_files_are_valid() {
        let themes = Path::new(env!("CARGO_MANIFEST_DIR")).join("themes");
        for theme in fs::read_dir(themes).unwrap().flatten() {
            let Ok(files) = fs::read_dir(theme.path().join("starter")) else {
                continue;
            };
            for file in files.flatten().map(|f| f.path()) {
                let text = fs::read_to_string(&file).unwrap();
                if file.extension().is_some_and(|e| e == "md") {
                    if let Err(e) = parse(&text) {
                        panic!("{}: {e}", file.display());
                    }
                } else if let Err(e) = toml::from_str::<SiteFile>(&text) {
                    panic!("{}: {e}", file.display());
                }
            }
        }
    }
}
