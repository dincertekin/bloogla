use serde::{Deserialize, Serialize};

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct Config {
    pub blog_name: String,
    pub admin_email: String,
    pub admin_password_hash: String,
    pub port: u16,
    pub production: bool,
    pub base_url: String,
}
