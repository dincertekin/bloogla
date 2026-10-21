//! Turning what authors write (Markdown with shortcodes) into web pages.
//!
//! - `markdown`: Markdown → safe HTML, plain text and excerpts
//! - `shortcodes`: `[youtube ...]`, `[toc]` and a theme's own shortcodes
//! - `seo`: meta tags, Open Graph and structured data for `<head>`
//! - `text`: slugs, escaping, URL encoding and dates

pub mod markdown;
pub mod seo;
pub mod shortcodes;
pub mod text;

use crate::app::state::AppState;

/// A post's Markdown as safe HTML, with shortcodes expanded: the built-in
/// ones and those the active theme defines in `shortcodes/<name>.html`.
pub async fn render(state: &AppState, markdown: &str) -> String {
    let html = markdown::to_safe_html(markdown);
    if !html.contains('[') {
        return html;
    }
    let active = crate::db::settings::load(&state.pool)
        .await
        .active_theme
        .clone();
    let Ok(themes) = state.themes.read() else {
        return html;
    };
    let tera = themes.pick(&active).map(|theme| theme.tera);
    shortcodes::expand(&html, &|name, args| {
        let tera = tera?;
        let template = format!("shortcodes/{name}.html");
        if !tera.get_template_names().any(|t| t == template) {
            return None;
        }
        let mut context = tera::Context::new();
        context.insert("args", &args.args);
        context.insert("params", &args.params);
        tera.render(&template, &context)
            .map_err(|e| tracing::error!("Shortcode {name} failed: {e:?}"))
            .ok()
    })
}

/// Make root-relative links absolute (`/uploads/a.jpg` → `https://site/uploads/a.jpg`),
/// for HTML that is read elsewhere: feed readers and emails.
pub fn absolute_links(html: &str, base_url: &str) -> String {
    html.replace("src=\"/", &format!("src=\"{base_url}/"))
        .replace("href=\"/", &format!("href=\"{base_url}/"))
}

/// A picture in Media: its file, its 800px and 400px copies, width and height.
type PictureCopies = (
    String,
    Option<String>,
    Option<String>,
    Option<i64>,
    Option<i64>,
);

/// Give the post's own pictures (from Media) their smaller copies, so phones
/// download a picture the size they show it: `srcset` with the 400px and
/// 800px copies, `sizes` for a text column, and the picture's width and
/// height so the page doesn't jump while it loads. Pictures from other
/// sites, or that already have a `srcset`, stay as they are. For the post
/// page only: feeds and exports want the plain addresses.
pub async fn with_image_copies(pool: &sqlx::SqlitePool, html: &str) -> String {
    const PREFIX: &str = "src=\"/uploads/";
    // The files the post shows, looked up in one query.
    let mut names: Vec<&str> = Vec::new();
    for (start, _) in html.match_indices(PREFIX) {
        let rest = &html[start + PREFIX.len()..];
        if let Some(end) = rest.find('"') {
            if !names.contains(&&rest[..end]) {
                names.push(&rest[..end]);
            }
        }
    }
    if names.is_empty() {
        return html.to_string();
    }
    let marks = vec!["?"; names.len()].join(", ");
    let sql = format!(
        "SELECT filename, small_filename, thumb_filename, width, height FROM media
         WHERE filename IN ({marks})"
    );
    let mut query = sqlx::query_as::<_, PictureCopies>(&sql);
    for name in &names {
        query = query.bind(*name);
    }
    let copies = crate::db::or_log(query.fetch_all(pool).await, "find picture copies");

    let mut out = String::with_capacity(html.len());
    let mut rest = html;
    while let Some(start) = rest.find("<img ") {
        let Some(length) = rest[start..].find('>') else {
            break;
        };
        // Without a closing `/` (as in `<img … />`), so additions go inside.
        let tag = rest[start..start + length].trim_end_matches('/').trim_end();
        let tag_end = start + length - (rest[start..start + length].len() - tag.len());
        out.push_str(&rest[..start]);
        let found = copies
            .iter()
            .find(|(name, ..)| tag.contains(&format!("{PREFIX}{name}\"")));
        match found {
            Some((name, small, thumb, width, height))
                if !tag.contains("srcset=") && thumb.is_some() =>
            {
                let mut sources: Vec<String> = Vec::new();
                if let Some(thumb) = thumb {
                    sources.push(format!("/uploads/{thumb} 400w"));
                }
                if let Some(small) = small {
                    sources.push(format!("/uploads/{small} 800w"));
                }
                if let Some(width) = width {
                    sources.push(format!("/uploads/{name} {width}w"));
                }
                out.push_str(tag);
                out.push_str(&format!(
                    " srcset=\"{}\" sizes=\"(max-width: 760px) 100vw, 740px\"",
                    sources.join(", ")
                ));
                if let (Some(width), Some(height)) = (width, height) {
                    if !tag.contains("width=") && !tag.contains("height=") {
                        out.push_str(&format!(" width=\"{width}\" height=\"{height}\""));
                    }
                }
            }
            _ => out.push_str(tag),
        }
        rest = &rest[tag_end..];
    }
    out.push_str(rest);
    out
}
