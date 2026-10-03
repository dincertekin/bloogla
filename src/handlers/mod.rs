//! Request handlers: the functions that answer each URL. `server/routes.rs`
//! says which handler answers which URL.
//!
//! - `site`: the public website (theme pages, feeds, comments, newsletter)
//! - `admin`: the admin panel
//! - `api`: the JSON API

pub mod admin;
pub mod api;
pub mod site;

use crate::app::state::AppState;

use axum::http::HeaderMap;
use std::net::{IpAddr, SocketAddr};

/// The visitor's address. Behind a local reverse proxy it comes from
/// `X-Forwarded-For`; otherwise that header could be forged, so it's ignored.
pub fn client_ip(state: &AppState, peer: SocketAddr, headers: &HeaderMap) -> IpAddr {
    if state.config.host.is_loopback() {
        let forwarded = headers
            .get("x-forwarded-for")
            .and_then(|v| v.to_str().ok())
            .and_then(|v| v.split(',').next())
            .and_then(|v| v.trim().parse().ok());
        if let Some(ip) = forwarded {
            return ip;
        }
    }
    peer.ip()
}
