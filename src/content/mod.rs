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
    let chosen = crate::services::themes::chosen_theme(&active);
    let Ok(themes) = state.themes.read() else {
        return html;
    };
    let tera = themes.pick(&chosen).map(|theme| theme.tera);
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
