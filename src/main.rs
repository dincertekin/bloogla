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
    #[cfg(debug_assertions)]
    run_in_main_folder_of_source();
    if let Err(e) = command.run().await {
        app::console::failed(&e.to_string());
        std::process::exit(1);
    }
}

/// Bloogla keeps its site (`data/`, `uploads/`, `themes/`) in the folder it
/// starts in. `cargo run` typed inside a folder of the source code, like
/// `themes/`, would make a second site there, and its `data` folder would
/// show up as a theme. So a development build started anywhere inside the
/// source code runs in its main folder instead. Release builds, and
/// development builds started outside the source code, are left alone.
#[cfg(debug_assertions)]
fn run_in_main_folder_of_source() {
    let source = std::path::Path::new(env!("CARGO_MANIFEST_DIR"));
    let Ok(here) = std::env::current_dir() else {
        return;
    };
    if here != source && here.starts_with(source) && std::env::set_current_dir(source).is_ok() {
        tracing::info!(
            "Development build started in {}: running in {} instead.",
            here.display(),
            source.display()
        );
    }
}
