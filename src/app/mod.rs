//! Building blocks the rest of Bloogla uses everywhere.
//!
//! - `console`: what a person sees in the terminal window (start screen, news, problems)
//! - `config`: settings read from `BLOOGLA_*` environment variables
//! - `state`: `AppState`, what every request handler can reach
//! - `models`: shared data types (Post, Tag, Role, CurrentUser...)
//! - `security`: password hashing, random tokens
//! - `secrets`: encrypting secrets stored in the database (key in data/secret.key)
//! - `totp`: two-factor login codes from authenticator apps

pub mod config;
pub mod console;
pub mod models;
pub mod private_files;
pub mod secrets;
pub mod security;
pub mod state;
pub mod totp;
