//! Work Bloogla does besides answering a page request: sending email,
//! backing up, handling themes and checking for updates.
//!
//! Request handlers (in `handlers/`) and commands (in `commands/`) call these.

pub mod backup;
pub mod email;
pub mod themes;
pub mod updates;
