#[derive(Debug, Clone)]
pub struct Config {
    pub database_url: String,
    pub port: u16,
    pub production: bool,
    pub base_url: String,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            database_url: "sqlite://data/bloogla.db?mode=rwc".to_string(),
            port: 8080,
            production: false,
            base_url: "http://localhost:8080".to_string(),
        }
    }
}
