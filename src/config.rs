use std::env;
use std::net::IpAddr;

#[derive(Debug, Clone)]
pub struct Config {
    pub database_url: String,
    pub host: IpAddr,
    pub port: u16,
    pub production: bool,
    pub base_url: String,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            database_url: "sqlite://data/bloogla.db?mode=rwc".to_string(),
            host: IpAddr::from([0, 0, 0, 0]),
            port: 8080,
            production: false,
            base_url: "http://localhost:8080".to_string(),
        }
    }
}

impl Config {
    /// Build the config from `BLOOGLA_*` environment variables, falling back to defaults.
    ///
    /// - `BLOOGLA_HOST`: bind address (default `0.0.0.0`; use `127.0.0.1` behind a reverse proxy)
    /// - `BLOOGLA_PORT`: listen port (default `8080`)
    /// - `BLOOGLA_BASE_URL`: public URL used in RSS, sitemap and CSRF checks
    /// - `BLOOGLA_PRODUCTION`: `true` enables secure (HTTPS-only) session cookies
    pub fn from_env() -> Result<Self, String> {
        let defaults = Self::default();

        let host = match env::var("BLOOGLA_HOST") {
            Ok(v) => v
                .parse()
                .map_err(|_| format!("BLOOGLA_HOST is not a valid IP address: {v}"))?,
            Err(_) => defaults.host,
        };

        let port = match env::var("BLOOGLA_PORT") {
            Ok(v) => v
                .parse()
                .map_err(|_| format!("BLOOGLA_PORT is not a valid port: {v}"))?,
            Err(_) => defaults.port,
        };

        let production = match env::var("BLOOGLA_PRODUCTION").as_deref() {
            Ok("1" | "true" | "yes") => true,
            Ok("0" | "false" | "no") | Err(_) => false,
            Ok(v) => {
                return Err(format!(
                    "BLOOGLA_PRODUCTION must be true or false, got: {v}"
                ))
            }
        };

        let base_url = env::var("BLOOGLA_BASE_URL")
            .unwrap_or_else(|_| format!("http://localhost:{port}"))
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
            ..defaults
        })
    }
}
