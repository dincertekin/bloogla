//! Work Bloogla does besides answering a page request: sending email (and
//! new posts to subscribers),
//! backing up (and copying backups to storage), preparing uploaded pictures and videos, importing from WordPress and
//! exporting, handling themes and checking for updates.
//!
//! Request handlers (in `handlers/`) and commands (in `commands/`) call these.

pub mod backup;
// Demo posts and pages for `cargo run`; left out of release builds.
#[cfg(debug_assertions)]
pub mod demo;
pub mod email;
pub mod export;
pub mod images;
pub mod import;
pub mod restore;
pub mod storage;
pub mod subscriptions;
pub mod themes;
pub mod updates;
pub mod videos;
