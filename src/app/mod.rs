//! Building blocks the rest of Bloogla uses everywhere.
//!
//! - `config`: settings read from `BLOOGLA_*` environment variables
//! - `state`: `AppState`, what every request handler can reach
//! - `models`: shared data types (Post, Tag, Role, CurrentUser...)
//! - `security`: password hashing, random tokens
//! - `secrets`: encrypting secrets stored in the database (key in data/secret.key)
//! - `totp`: two-factor login codes from authenticator apps

pub mod config;
pub mod models;
pub mod secrets;
pub mod security;
pub mod state;
pub mod totp;
