//! Built-in HTTPS with automatic Let's Encrypt certificates.
//!
//! Certificates are requested with the TLS-ALPN-01 challenge on the HTTPS port
//! (so port 443 must be reachable from the internet), renewed automatically,
//! and cached in `data/acme/`. Plain HTTP on port 80 redirects to HTTPS.

use crate::app::config::TlsConfig;

use axum::extract::Request;
use axum::http::{header, StatusCode};
use axum::response::{IntoResponse, Redirect, Response};
use axum::Router;
use futures_util::StreamExt;
use rustls_acme::caches::DirCache;
use rustls_acme::AcmeConfig;
use std::future::Future;
use std::net::SocketAddr;
use std::time::Duration;

pub const CERT_CACHE_DIR: &str = "data/acme";

/// Serve `app` over HTTPS until `shutdown` resolves.
pub async fn serve(
    app: Router,
    addr: SocketAddr,
    tls: &TlsConfig,
    shutdown: impl Future<Output = ()> + Send + 'static,
) -> Result<(), Box<dyn std::error::Error>> {
    if addr.port() != 443 {
        tracing::warn!(
            "HTTPS is on port {}; Let's Encrypt validates on port 443, so forward 443 to it",
            addr.port()
        );
    }

    let mut acme = AcmeConfig::new(tls.domains.clone())
        .contact(tls.email.iter().map(|e| format!("mailto:{e}")))
        .cache(DirCache::new(CERT_CACHE_DIR))
        .directory_lets_encrypt(!tls.staging)
        .state();
    let acceptor = acme.axum_acceptor(acme.default_rustls_config());

    tokio::spawn(async move {
        while let Some(event) = acme.next().await {
            match event {
                Ok(ok) => tracing::info!("HTTPS certificate: {ok:?}"),
                Err(err) => tracing::error!("HTTPS certificate error: {err:?}"),
            }
        }
    });

    spawn_http_redirect(
        SocketAddr::new(addr.ip(), tls.http_port),
        tls.domains.clone(),
    )
    .await?;

    let handle = axum_server::Handle::new();
    let shutdown_handle = handle.clone();
    tokio::spawn(async move {
        shutdown.await;
        shutdown_handle.graceful_shutdown(Some(Duration::from_secs(10)));
    });

    tracing::info!(
        "Serving HTTPS on {addr} for {} (certificates in {CERT_CACHE_DIR}/)",
        tls.domains.join(", ")
    );
    axum_server::bind(addr)
        .handle(handle)
        .acceptor(acceptor)
        .serve(app.into_make_service_with_connect_info::<SocketAddr>())
        .await?;
    Ok(())
}

/// Answer plain HTTP with a permanent redirect to the HTTPS version.
async fn spawn_http_redirect(
    addr: SocketAddr,
    domains: Vec<String>,
) -> Result<(), Box<dyn std::error::Error>> {
    let listener = tokio::net::TcpListener::bind(addr)
        .await
        .map_err(|e| format!("Could not listen on {addr} for HTTP redirects: {e}"))?;

    let redirect = Router::new().fallback(move |req: Request| {
        let domains = domains.clone();
        async move { https_redirect(&req, &domains) }
    });

    tokio::spawn(async move {
        if let Err(e) = axum::serve(listener, redirect).await {
            tracing::error!("HTTP redirect server stopped: {e}");
        }
    });
    tracing::info!("Redirecting HTTP on {addr} to HTTPS");
    Ok(())
}

/// Only configured domains are redirected to, so this can't be used as an open redirect.
fn https_redirect(req: &Request, domains: &[String]) -> Response {
    let requested = req
        .headers()
        .get(header::HOST)
        .and_then(|h| h.to_str().ok())
        .map(|h| h.split(':').next().unwrap_or(h).to_ascii_lowercase());
    let host = requested
        .filter(|h| domains.contains(h))
        .unwrap_or_else(|| domains[0].clone());

    let path = req
        .uri()
        .path_and_query()
        .map(|p| p.as_str())
        .unwrap_or("/");
    if !path.starts_with('/') {
        return StatusCode::BAD_REQUEST.into_response();
    }
    Redirect::permanent(&format!("https://{host}{path}")).into_response()
}

#[cfg(test)]
mod tests {
    use super::https_redirect;
    use axum::body::Body;
    use axum::http::{header, Request};

    fn location(host: &str, uri: &str) -> String {
        let req = Request::builder()
            .uri(uri)
            .header(header::HOST, host)
            .body(Body::empty())
            .unwrap();
        let domains = vec!["example.com".to_string(), "www.example.com".to_string()];
        https_redirect(&req, &domains).headers()[header::LOCATION]
            .to_str()
            .unwrap()
            .to_string()
    }

    #[test]
    fn redirects_only_to_configured_domains() {
        assert_eq!(
            location("www.example.com", "/post/a?x=1"),
            "https://www.example.com/post/a?x=1"
        );
        assert_eq!(location("example.com:80", "/"), "https://example.com/");
        assert_eq!(location("evil.com", "/login"), "https://example.com/login");
    }
}
