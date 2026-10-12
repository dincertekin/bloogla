//! Request handlers: the functions that answer each URL. `server/routes.rs`
//! says which handler answers which URL.
//!
//! - `site`: the public website (theme pages, feeds, comments)
//! - `admin`: the admin panel

pub mod admin;
pub mod site;

use crate::app::state::AppState;

use axum::http::HeaderMap;
use axum::response::Html;
use std::net::{IpAddr, SocketAddr};

/// A small, self-contained page for when the theme can't draw one (the
/// theme is broken, or something crashed). `title` and `text` are English
/// texts that get translated; `link` is an optional `(address, label)`.
pub fn plain_page(
    lang: crate::i18n::Lang,
    title: &'static str,
    text: &'static str,
    link: Option<(&str, &'static str)>,
) -> Html<String> {
    let link = link
        .map(|(href, label)| {
            format!(
                r#"<p><a href="{}">{}</a></p>"#,
                crate::content::text::escape_html(href),
                lang.t(label)
            )
        })
        .unwrap_or_default();
    Html(format!(
        r#"<!doctype html><html lang="{}"><head><meta charset="utf-8"><meta name="viewport" content="width=device-width, initial-scale=1"><meta name="robots" content="noindex"><title>{title}</title>
<style>body{{margin:0;min-height:100vh;display:grid;place-items:center;padding:1.5rem;box-sizing:border-box;font:16px/1.6 system-ui,-apple-system,"Segoe UI",sans-serif;color:#111827;background:#fafafa}}main{{max-width:30rem}}h1{{font-size:1.5rem;line-height:1.3;margin:0 0 .75rem}}p{{margin:0 0 1rem;color:#4b5563}}a{{color:#111827;font-weight:600}}@media (prefers-color-scheme:dark){{body{{color:#f3f4f6;background:#111827}}p{{color:#d1d5db}}a{{color:#f3f4f6}}}}</style>
</head><body><main><h1>{title}</h1><p>{text}</p>{link}</main></body></html>"#,
        lang.code(),
        title = lang.t(title),
        text = lang.t(text),
    ))
}

/// The visitor's address. Behind a local reverse proxy it comes from
/// `X-Forwarded-For`; otherwise that header could be forged, so it's ignored.
pub fn client_ip(state: &AppState, peer: SocketAddr, headers: &HeaderMap) -> IpAddr {
    if state.config.host.is_loopback() {
        if let Some(ip) = forwarded_for(headers) {
            return ip;
        }
    }
    peer.ip()
}

/// The visitor's address from `X-Forwarded-For`, trusting only the proxy.
///
/// A visitor can send this header themselves, so it arrives with the address
/// they wrote first and the one the local proxy added last (at the end of the
/// line, or on a line of its own). The proxy is the only part we trust, so we
/// read its entry: the last one. (Reading the first let a visitor write any
/// address there and slip past the rate limits by changing it on every
/// request.)
///
/// Only call this when the proxy is trusted (Bloogla is on a loopback address).
pub fn forwarded_for(headers: &HeaderMap) -> Option<IpAddr> {
    headers
        .get_all("x-forwarded-for")
        .iter()
        .next_back()
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.rsplit(',').next())
        .and_then(|v| v.trim().parse().ok())
}
