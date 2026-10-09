//! Bloogla: a fast, single-binary blog engine.
//!
//! Start reading here. `main` reads the command (`commands/`), `server/` runs
//! the website, and `server/routes.rs` lists every URL with the code that
//! answers it. The README has a map of every folder.

mod app;
mod commands;
mod content;
mod db;
mod handlers;
mod i18n;
mod server;
mod services;

#[cfg(test)]
mod tests;

use commands::Command;

#[tokio::main]
async fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let Some(command) = Command::parse(&args) else {
        eprint!("Unknown command: {}\n\n{}", args.join(" "), commands::HELP);
        std::process::exit(2);
    };

    server::init_logging();
    if let Err(e) = command.run().await {
        app::console::failed(&e.to_string());
        std::process::exit(1);
    }
}
