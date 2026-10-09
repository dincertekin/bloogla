use std::env;
use std::net::IpAddr;

#[derive(Debug, Clone)]
pub struct Config {
    pub database_url: String,
    pub host: IpAddr,
    pub port: u16,
    pub production: bool,
    pub base_url: String,
    /// Built-in HTTPS; `None` when serving plain HTTP (e.g. behind a reverse proxy).
    pub tls: Option<TlsConfig>,
}

/// Automatic Let's Encrypt certificates for the given domains.
#[derive(Debug, Clone)]
pub struct TlsConfig {
    pub domains: Vec<String>,
    pub email: Option<String>,
    /// Use Let's Encrypt's staging server (untrusted certificates, generous limits).
    pub staging: bool,
    /// Port that redirects plain HTTP to HTTPS.
    pub http_port: u16,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            database_url: "sqlite://data/bloogla.db?mode=rwc".to_string(),
            host: IpAddr::from([0, 0, 0, 0]),
            port: 8080,
            production: false,
            base_url: "http://localhost:8080".to_string(),
            tls: None,
        }
    }
}

fn parse_bool(name: &str, default: bool) -> Result<bool, String> {
    match env::var(name).as_deref() {
        Ok("1" | "true" | "yes") => Ok(true),
        Ok("0" | "false" | "no") => Ok(false),
        Err(_) => Ok(default),
        Ok(v) => Err(format!("{name} must be true or false, got: {v}")),
    }
}

fn parse_port(name: &str, default: u16) -> Result<u16, String> {
    match env::var(name) {
        Ok(v) => v
            .parse()
            .map_err(|_| format!("{name} is not a valid port: {v}")),
        Err(_) => Ok(default),
    }
}

impl Config {
    /// Build the config from `BLOOGLA_*` environment variables, falling back to defaults.
    ///
    /// - `BLOOGLA_HOST`: bind address (default `0.0.0.0`; use `127.0.0.1` behind a reverse proxy)
    /// - `BLOOGLA_PORT`: listen port (default `8080`, or `443` with built-in HTTPS)
    /// - `BLOOGLA_BASE_URL`: public URL used in RSS, sitemap and CSRF checks
    /// - `BLOOGLA_PRODUCTION`: `true` enables secure (HTTPS-only) session cookies
    /// - `BLOOGLA_TLS_DOMAINS`: comma-separated domains; turns on built-in HTTPS
    /// - `BLOOGLA_TLS_EMAIL`: contact for Let's Encrypt expiry notices
    /// - `BLOOGLA_TLS_STAGING`: `true` to test against Let's Encrypt staging
    /// - `BLOOGLA_HTTP_PORT`: port redirecting to HTTPS (default `80`)
    pub fn from_env() -> Result<Self, String> {
        let defaults = Self::default();

        let host = match env::var("BLOOGLA_HOST") {
            Ok(v) => v
                .parse()
                .map_err(|_| format!("BLOOGLA_HOST is not a valid IP address: {v}"))?,
            Err(_) => defaults.host,
        };

        let domains: Vec<String> = env::var("BLOOGLA_TLS_DOMAINS")
            .unwrap_or_default()
            .split(',')
            .map(|d| d.trim().to_ascii_lowercase())
            .filter(|d| !d.is_empty())
            .collect();
        if let Some(bad) = domains
            .iter()
            .find(|d| d.contains(['/', ':', ' ']) || !d.contains('.'))
        {
            return Err(format!(
                "BLOOGLA_TLS_DOMAINS should list bare domains like example.com, got: {bad}"
            ));
        }
        let tls = if domains.is_empty() {
            None
        } else {
            Some(TlsConfig {
                email: env::var("BLOOGLA_TLS_EMAIL")
                    .ok()
                    .filter(|e| e.contains('@')),
                staging: parse_bool("BLOOGLA_TLS_STAGING", false)?,
                http_port: parse_port("BLOOGLA_HTTP_PORT", 80)?,
                domains,
            })
        };

        let port = parse_port(
            "BLOOGLA_PORT",
            if tls.is_some() { 443 } else { defaults.port },
        )?;
        let production = parse_bool("BLOOGLA_PRODUCTION", tls.is_some())?;

        let default_base_url = match &tls {
            Some(tls) => format!("https://{}", tls.domains[0]),
            None => format!("http://localhost:{port}"),
        };
        let base_url = env::var("BLOOGLA_BASE_URL")
            .unwrap_or(default_base_url)
            .trim_end_matches('/')
            .to_string();

        if !base_url.starts_with("http://") && !base_url.starts_with("https://") {
            return Err(format!(
                "BLOOGLA_BASE_URL must start with http:// or https://, got: {base_url}"
            ));
        }

        Ok(Self {
            host,
            port,
            production,
            base_url,
            tls,
            ..defaults
        })
    }
}
