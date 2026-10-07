//! The public website: pages drawn by the active theme, feeds, and the forms
//! visitors can send (comments, newsletter sign-ups, webmentions).

mod analytics;
pub mod comments;
pub mod feeds;
pub mod newsletter;
pub mod pages;
pub mod webmention;

use crate::app::state::AppState;
use crate::content::seo::{render_head, SeoKind, SeoMeta, SiteInfo};
use crate::db::settings::{self, Settings};

use axum::http::{HeaderMap, StatusCode};
use axum::response::Response;
use std::collections::{HashMap, VecDeque};
use std::net::{IpAddr, SocketAddr};
use std::sync::Mutex;
use std::time::{Duration, Instant};

/// Form submissions (comments, sign-ups, webmentions) a visitor may send per
/// [`RATE_WINDOW`].
const RATE_LIMIT: usize = 5;
const RATE_WINDOW: Duration = Duration::from_secs(10 * 60);

/// A published page for theme menus and sidebars (`pages` in templates).
#[derive(serde::Serialize)]
struct PageLink {
    title: String,
    url: String,
    slug: String,
    fields: crate::db::fields::Fields,
}

/// Context every theme template receives: site info, menu, tags, pages and
/// default SEO tags.
async fn base_context(state: &AppState, site: &Settings) -> tera::Context {
    let tags = crate::db::tags::all(&state.pool).await;
    let pages: Vec<PageLink> = crate::db::posts::public_pages(&state.pool)
        .await
        .into_iter()
        .map(|page| PageLink {
            url: crate::server::routes::post_path(&page.slug, true),
            title: page.title,
            slug: page.slug,
            fields: page.fields,
        })
        .collect();
    let seo_head = seo_head(
        state,
        site,
        &SeoMeta {
            title: &site.blog_name,
            description: &site.blog_description,
            canonical_url: &state.config.base_url,
            image: None,
            kind: SeoKind::Website,
            noindex: false,
        },
    );

    let mut context = tera::Context::new();
    context.insert("blog_name", &site.blog_name);
    context.insert("blog_description", &site.blog_description);
    context.insert("base_url", &state.config.base_url);
    context.insert("menu", &site.menu());
    context.insert("tags", &tags);
    context.insert("pages", &pages);
    context.insert("search_query", "");
    context.insert("show_views", &site.show_views);
    context.insert("newsletter_enabled", &site.newsletter);
    context.insert("lang", site.language.code());
    context.insert("t", &site.language.theme_strings());
    context.insert("seo_head", &seo_head);
    context
}

/// The SEO `<head>` block for a page.
fn seo_head(state: &AppState, site: &Settings, meta: &SeoMeta) -> String {
    render_head(
        meta,
        &SiteInfo {
            name: &site.blog_name,
            base_url: &state.config.base_url,
            publisher_type: &site.publisher_type,
            publisher_name: site.publisher(),
            icon: (!site.site_icon.is_empty()).then_some(site.site_icon.as_str()),
        },
    )
}

/// SEO tags for pages search engines should skip (search results, 404s, notices).
fn noindex_head(state: &AppState, site: &Settings) -> String {
    seo_head(
        state,
        site,
        &SeoMeta {
            title: &site.blog_name,
            description: "",
            canonical_url: &state.config.base_url,
            image: None,
            kind: SeoKind::Website,
            noindex: true,
        },
    )
}

/// Themed 404 page (plain HTML when the theme has no `404.html`).
pub async fn not_found(state: &AppState) -> Response {
    let site = settings::load(&state.pool).await;
    let mut context = base_context(state, &site).await;
    context.insert("seo_head", &noindex_head(state, &site));
    crate::services::themes::render(
        state,
        &["templates/404.html"],
        context,
        StatusCode::NOT_FOUND,
    )
    .await
}

/// A short themed page with a title, a sentence, and optionally a one-button
/// form `(action, token, label)`. Uses the theme's `message.html`.
pub async fn render_message(
    state: &AppState,
    status: StatusCode,
    title: &str,
    text: &str,
    button: Option<(&str, &str, &str)>,
) -> Response {
    let site = settings::load(&state.pool).await;
    let lang = site.language;
    let mut context = base_context(state, &site).await;
    context.insert("seo_head", &noindex_head(state, &site));
    context.insert("message_title", &lang.t_owned(title));
    context.insert("message_text", &lang.t_owned(text));
    if let Some((action, token, label)) = button {
        context.insert("message_action", action);
        context.insert("message_token", token);
        context.insert("message_button", &lang.t_owned(label));
    }
    crate::services::themes::render(state, &["templates/message.html"], context, status).await
}

/// Recent form submissions per visitor address.
static RECENT: Mutex<Option<HashMap<IpAddr, VecDeque<Instant>>>> = Mutex::new(None);

/// Rate limit shared by public forms. Records a submission from the visitor
/// and returns false when they've sent too many lately.
pub fn allow_from(state: &AppState, peer: SocketAddr, headers: &HeaderMap) -> bool {
    let ip = crate::handlers::client_ip(state, peer, headers);
    let Ok(mut guard) = RECENT.lock() else {
        return true;
    };
    let map = guard.get_or_insert_with(HashMap::new);
    let now = Instant::now();
    map.retain(|_, times| {
        times.retain(|t| now.duration_since(*t) < RATE_WINDOW);
        !times.is_empty()
    });
    let times = map.entry(ip).or_default();
    if times.len() >= RATE_LIMIT {
        return false;
    }
    times.push_back(now);
    true
}
