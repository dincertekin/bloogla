//! Building blocks the rest of Bloogla uses everywhere.
//!
//! - `config`: settings read from `BLOOGLA_*` environment variables
//! - `state`: `AppState`, what every request handler can reach
//! - `models`: shared data types (Post, Tag, Role, CurrentUser...)
//! - `security`: password hashing, random tokens, API token hashing

pub mod config;
pub mod models;
pub mod security;
pub mod state;
