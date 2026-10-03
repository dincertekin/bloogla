//! Webmentions (https://www.w3.org/TR/webmention/): sites notify each other
//! when they link to one another.
//!
//! Receiving: `POST /webmention` with `source` and `target`. The source page is
//! fetched in the background; if it really links to one of our posts, the
//! mention is added to the comment queue for approval.
//!
//! Sending: when a post is published, each external page it links to is
//! checked for a webmention endpoint and notified once.
//!
//! Every fetch refuses private and local network addresses (and re-checks
//! after redirects), so these requests can't be aimed at the server's own
//! network.

use crate::app::state::AppState;
use crate::db::posts::PUBLIC_POST_FILTER;
use crate::db::{or_log, settings};

use reqwest::Url;
use std::net::IpAddr;
use std::time::Duration;

const MAX_BODY: usize = 1024 * 1024;
const MAX_REDIRECTS: usize = 3;
const TIMEOUT: Duration = Duration::from_secs(10);
const MAX_TARGETS_PER_POST: usize = 20;
const USER_AGENT: &str = concat!("Bloogla/", env!("CARGO_PKG_VERSION"), " (webmention)");

/// For local development only: allow fetching from private addresses.
fn allow_local() -> bool {
    std::env::var("BLOOGLA_WEBMENTION_ALLOW_LOCAL").as_deref() == Ok("true")
}

/// Addresses on the public internet (not loopback, private, link-local...).
fn is_public(ip: IpAddr) -> bool {
    match ip {
        IpAddr::V4(v4) => {
            let [a, b, ..] = v4.octets();
            !(v4.is_private()
                || v4.is_loopback()
                || v4.is_link_local()
                || v4.is_unspecified()
                || v4.is_broadcast()
                || v4.is_documentation()
                || a == 0
                || (a == 100 && (64..=127).contains(&b)) // carrier-grade NAT
                || a >= 224) // multicast and reserved
        }
        IpAddr::V6(v6) => {
            if let Some(v4) = v6.to_ipv4_mapped() {
                return is_public(IpAddr::V4(v4));
            }
            let first = v6.segments()[0];
            !(v6.is_loopback()
                || v6.is_unspecified()
                || (first & 0xfe00) == 0xfc00 // unique local
                || (first & 0xffc0) == 0xfe80 // link-local
                || (first & 0xff00) == 0xff00) // multicast
        }
    }
}

/// A fetched page: final URL, `Link` headers and body text.
struct Page {
    url: Url,
    links: Vec<String>,
    body: String,
}

/// GET or POST `url` safely: public addresses only, limited redirects, size and time.
async fn safe_request(mut url: Url, form: Option<&[(&str, &str)]>) -> Result<Page, String> {
    for _ in 0..=MAX_REDIRECTS {
        if !matches!(url.scheme(), "http" | "https") {
            return Err("only http and https URLs".into());
        }
        let host = url.host_str().ok_or("URL has no host")?.to_string();
        let port = url.port_or_known_default().unwrap_or(443);

        let addrs: Vec<_> = tokio::net::lookup_host((host.as_str(), port))
            .await
            .map_err(|e| format!("can't resolve {host}: {e}"))?
            .collect();
        let addr = addrs
            .iter()
            .find(|a| allow_local() || is_public(a.ip()))
            .copied()
            .ok_or_else(|| format!("{host} isn't a public address"))?;

        // Pin the connection to the address we just checked.
        let client = reqwest::Client::builder()
            .resolve(&host, addr)
            .redirect(reqwest::redirect::Policy::none())
            .timeout(TIMEOUT)
            .user_agent(USER_AGENT)
            .build()
            .map_err(|e| e.to_string())?;

        let request = match form {
            Some(fields) => client.post(url.clone()).form(fields),
            None => client.get(url.clone()),
        };
        let mut response = request.send().await.map_err(|e| e.to_string())?;

        if response.status().is_redirection() {
            let location = response
                .headers()
                .get(reqwest::header::LOCATION)
                .and_then(|l| l.to_str().ok())
                .ok_or("redirect without a location")?;
            url = url.join(location).map_err(|e| e.to_string())?;
            continue;
        }

        let status = response.status();
        let links = response
            .headers()
            .get_all(reqwest::header::LINK)
            .iter()
            .filter_map(|v| v.to_str().ok().map(str::to_string))
            .collect();
        let mut body = Vec::new();
        while let Some(chunk) = response.chunk().await.map_err(|e| e.to_string())? {
            body.extend_from_slice(&chunk);
            if body.len() > MAX_BODY {
                return Err("page too large".into());
            }
        }
        if !status.is_success() {
            return Err(format!("status {status}"));
        }
        return Ok(Page {
            url,
            links,
            body: String::from_utf8_lossy(&body).into_owned(),
        });
    }
    Err("too many redirects".into())
}

/// Check a received mention in the background: fetch `source` and, if it
/// really links to the post, add it to the comment queue.
pub fn check_in_background(state: &AppState, post_id: i64, source: Url, target: Url) {
    let state = state.clone();
    tokio::spawn(async move {
        if let Err(e) = verify(&state, post_id, source.clone(), &target).await {
            tracing::info!("Webmention from {source} not accepted: {e}");
        }
    });
}

/// The public post a target URL points at, if it's on this site.
pub async fn target_post(state: &AppState, target: &Url) -> Option<i64> {
    let base = Url::parse(&state.config.base_url).ok()?;
    if target.host_str() != base.host_str() {
        return None;
    }
    let slug = target.path().trim_end_matches('/').strip_prefix("/post/")?;
    let slug = crate::content::text::percent_decode(slug);
    or_log(
        sqlx::query_scalar(&format!(
            "SELECT id FROM posts WHERE slug = ? AND is_page = 0 AND {PUBLIC_POST_FILTER}"
        ))
        .bind(slug)
        .fetch_optional(&state.pool)
        .await,
        "webmention target",
    )
}

/// Fetch the source; store (or update) the mention if it links to the target,
/// remove it if it no longer does.
async fn verify(state: &AppState, post_id: i64, source: Url, target: &Url) -> Result<(), String> {
    let page = safe_request(source.clone(), None).await?;
    let target_str = target.as_str().trim_end_matches('/');
    let links_here = page.body.contains(&format!("href=\"{target_str}\""))
        || page.body.contains(&format!("href=\"{target_str}/\""))
        || page.body.contains(&format!("href='{target_str}'"));

    if !links_here {
        sqlx::query("DELETE FROM comments WHERE post_id = ? AND source_url = ?")
            .bind(post_id)
            .bind(source.as_str())
            .execute(&state.pool)
            .await
            .map_err(|e| e.to_string())?;
        return Err("source doesn't link to the target".into());
    }

    let author = microformat_text(&page.body, "p-author")
        .or_else(|| microformat_text(&page.body, "h-card"))
        .unwrap_or_else(|| page.url.host_str().unwrap_or("A website").to_string());
    let title = tag_text(&page.body, "title").unwrap_or_default();
    let content = microformat_text(&page.body, "p-summary")
        .or_else(|| meta_content(&page.body, "description"))
        .unwrap_or(title);
    let content = format!(
        "{}\n\n(Mentioned at {})",
        truncate(&content, 600),
        source.as_str()
    );

    sqlx::query(
        "INSERT INTO comments (post_id, author_name, content, status, source_url)
         VALUES (?, ?, ?, 'pending', ?)
         ON CONFLICT(post_id, source_url) WHERE source_url IS NOT NULL
         DO UPDATE SET author_name = excluded.author_name, content = excluded.content",
    )
    .bind(post_id)
    .bind(truncate(&author, 80))
    .bind(&content)
    .bind(source.as_str())
    .execute(&state.pool)
    .await
    .map_err(|e| e.to_string())?;
    tracing::info!("Webmention from {source} waiting for approval");
    let title: String = or_log(
        sqlx::query_scalar("SELECT title FROM posts WHERE id = ?")
            .bind(post_id)
            .fetch_one(&state.pool)
            .await,
        "webmention post title",
    );
    crate::services::email::notify_new_comment(
        state,
        title,
        format!("{} (webmention)", truncate(&author, 80)),
        truncate(&content, 300),
    );
    Ok(())
}

fn truncate(text: &str, max: usize) -> String {
    let text = text.split_whitespace().collect::<Vec<_>>().join(" ");
    if text.chars().count() <= max {
        text
    } else {
        format!("{}…", text.chars().take(max).collect::<String>())
    }
}

/// Text inside the first element whose class list contains `class`.
fn microformat_text(html: &str, class: &str) -> Option<String> {
    let mut rest = html;
    while let Some(pos) = rest.find("class=\"") {
        let after = &rest[pos + 7..];
        let classes = after.split('"').next()?;
        if classes.split_whitespace().any(|c| c == class) {
            let content = &after[after.find('>')? + 1..];
            // Take text up to the end of this element (approximate: next closing tag of a block).
            let end = content
                .find("</a>")
                .into_iter()
                .chain(content.find("</span>"))
                .chain(content.find("</div>"))
                .chain(content.find("</p>"))
                .min()
                .unwrap_or(content.len().min(300));
            let text = strip_tags(&content[..end]);
            if !text.is_empty() {
                return Some(decode(&text));
            }
        }
        rest = after;
    }
    None
}

fn tag_text(html: &str, tag: &str) -> Option<String> {
    let start = html.find(&format!("<{tag}"))?;
    let content = &html[start..];
    let content = &content[content.find('>')? + 1..];
    let end = content.find(&format!("</{tag}>"))?;
    Some(decode(strip_tags(&content[..end]).trim()))
}

fn meta_content(html: &str, name: &str) -> Option<String> {
    let marker = format!("name=\"{name}\"");
    let pos = html.find(&marker)?;
    let tag_start = html[..pos].rfind('<')?;
    let tag = &html[tag_start..tag_start + html[tag_start..].find('>')?];
    let content = tag.split("content=\"").nth(1)?.split('"').next()?;
    Some(decode(content))
}

fn strip_tags(html: &str) -> String {
    let mut text = String::new();
    let mut in_tag = false;
    for c in html.chars() {
        match c {
            '<' => in_tag = true,
            '>' => in_tag = false,
            _ if !in_tag => text.push(c),
            _ => {}
        }
    }
    text.split_whitespace().collect::<Vec<_>>().join(" ")
}

fn decode(text: &str) -> String {
    text.replace("&quot;", "\"")
        .replace("&#39;", "'")
        .replace("&#x27;", "'")
        .replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&nbsp;", " ")
        .replace("&amp;", "&")
}

/// Find a page's webmention endpoint from its `Link` header or HTML.
fn discover_endpoint(page: &Page) -> Option<Url> {
    for header in &page.links {
        for part in header.split(',') {
            let Some((url_part, params)) = part.split_once(';') else {
                continue;
            };
            let rel_is_webmention = params
                .split(';')
                .filter_map(|p| p.trim().strip_prefix("rel="))
                .any(|rel| {
                    rel.trim_matches('"')
                        .split_whitespace()
                        .any(|r| r == "webmention")
                });
            if rel_is_webmention {
                let href = url_part
                    .trim()
                    .trim_start_matches('<')
                    .trim_end_matches('>');
                return page.url.join(href).ok();
            }
        }
    }

    // <link rel="webmention" href="..."> or <a rel="webmention" href="...">
    let mut rest = page.body.as_str();
    while let Some(pos) = rest.find('<') {
        rest = &rest[pos + 1..];
        let Some(end) = rest.find('>') else { break };
        let tag = &rest[..end];
        if !(tag.starts_with("link ") || tag.starts_with("a ")) {
            continue;
        }
        let rel = attribute(tag, "rel").unwrap_or_default();
        if rel.split_whitespace().any(|r| r == "webmention") {
            let href = attribute(tag, "href").unwrap_or_default();
            return page.url.join(&decode(&href)).ok();
        }
    }
    None
}

fn attribute(tag: &str, name: &str) -> Option<String> {
    for quote in ['"', '\''] {
        let marker = format!("{name}={quote}");
        if let Some(pos) = tag.find(&marker) {
            let value = &tag[pos + marker.len()..];
            return value.split(quote).next().map(str::to_string);
        }
    }
    None
}

/// External links in rendered post HTML.
fn external_links(html: &str, own_host: &str) -> Vec<Url> {
    let mut links = Vec::new();
    for part in html.split("href=\"").skip(1) {
        let Some(href) = part.split('"').next() else {
            continue;
        };
        let Ok(url) = Url::parse(&decode(href)) else {
            continue;
        };
        if matches!(url.scheme(), "http" | "https")
            && url.host_str().is_some_and(|h| h != own_host)
            && !links.contains(&url)
        {
            links.push(url);
        }
    }
    links.truncate(MAX_TARGETS_PER_POST);
    links
}

/// Notify sites linked from a published post (each link once).
pub fn send_for_post(state: &AppState, post_id: i64) {
    let state = state.clone();
    tokio::spawn(async move {
        if !settings::load(&state.pool).await.send_webmentions {
            return;
        }
        let row: Option<(String, String)> = or_log(
            sqlx::query_as(&format!(
                "SELECT slug, content FROM posts WHERE id = ? AND is_page = 0 AND {PUBLIC_POST_FILTER}"
            ))
            .bind(post_id)
            .fetch_optional(&state.pool)
            .await,
            "webmention source post",
        );
        let Some((slug, content)) = row else { return };

        let source = format!(
            "{}{}",
            state.config.base_url,
            crate::server::routes::post_path(&slug, false)
        );
        let own_host = Url::parse(&state.config.base_url)
            .ok()
            .and_then(|u| u.host_str().map(str::to_string))
            .unwrap_or_default();
        let html = crate::content::markdown::to_safe_html(&content);

        for target in external_links(&html, &own_host) {
            let already: Option<i64> = or_log(
                sqlx::query_scalar(
                    "SELECT 1 FROM webmentions_sent WHERE post_id = ? AND target = ?",
                )
                .bind(post_id)
                .bind(target.as_str())
                .fetch_optional(&state.pool)
                .await,
                "webmention sent check",
            );
            if already.is_some() {
                continue;
            }

            let result = match send_one(&source, &target).await {
                Ok(true) => "sent".to_string(),
                Ok(false) => "no endpoint".to_string(),
                Err(e) => format!("failed: {e}"),
            };
            tracing::info!("Webmention to {target}: {result}");
            let _ = sqlx::query(
                "INSERT OR REPLACE INTO webmentions_sent (post_id, target, result) VALUES (?, ?, ?)",
            )
            .bind(post_id)
            .bind(target.as_str())
            .bind(&result)
            .execute(&state.pool)
            .await;
        }
    });
}

/// Ok(false) when the target doesn't accept webmentions.
async fn send_one(source: &str, target: &Url) -> Result<bool, String> {
    let page = safe_request(target.clone(), None).await?;
    let Some(endpoint) = discover_endpoint(&page) else {
        return Ok(false);
    };
    safe_request(
        endpoint,
        Some(&[("source", source), ("target", target.as_str())]),
    )
    .await?;
    Ok(true)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn blocks_private_and_local_addresses() {
        for ip in [
            "127.0.0.1",
            "10.0.0.5",
            "192.168.1.1",
            "172.16.0.1",
            "169.254.169.254",
            "100.64.0.1",
            "0.0.0.0",
            "::1",
            "fd00::1",
            "fe80::1",
            "::ffff:127.0.0.1",
        ] {
            assert!(!is_public(ip.parse().unwrap()), "{ip} should be blocked");
        }
        for ip in ["1.1.1.1", "93.184.216.34", "2606:4700::1111"] {
            assert!(is_public(ip.parse().unwrap()), "{ip} should be allowed");
        }
    }

    fn page(links: Vec<&str>, body: &str) -> Page {
        Page {
            url: Url::parse("https://blog.example/post/a").unwrap(),
            links: links.into_iter().map(str::to_string).collect(),
            body: body.to_string(),
        }
    }

    #[test]
    fn discovers_endpoints() {
        let from_header = page(vec![r#"<https://hooks.example/wm>; rel="webmention""#], "");
        assert_eq!(
            discover_endpoint(&from_header).unwrap().as_str(),
            "https://hooks.example/wm"
        );
        let from_html = page(
            vec![],
            r#"<head><link rel="webmention" href="/wm?x=1&amp;y=2"></head>"#,
        );
        assert_eq!(
            discover_endpoint(&from_html).unwrap().as_str(),
            "https://blog.example/wm?x=1&y=2"
        );
        assert!(discover_endpoint(&page(vec![], "<p>nothing</p>")).is_none());
    }

    #[test]
    fn finds_external_links_once() {
        let html = r#"<a href="https://a.example/x">a</a> <a href="https://me.example/post/b">self</a>
            <a href="https://a.example/x">again</a> <a href="mailto:x@y">mail</a>"#;
        let links = external_links(html, "me.example");
        assert_eq!(links.len(), 1);
        assert_eq!(links[0].as_str(), "https://a.example/x");
    }

    #[test]
    fn reads_author_from_microformats() {
        let html = r#"<div class="h-entry"><a class="p-author h-card" href="/">Jane Doe</a></div>"#;
        assert_eq!(
            microformat_text(html, "p-author").as_deref(),
            Some("Jane Doe")
        );
    }
}
