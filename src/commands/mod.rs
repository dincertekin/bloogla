//! The `bloogla` command line: reading what to do from the arguments and
//! running it. `serve` starts the website (see `server/`); the other commands
//! each have a file here.

mod backup;
mod disable_two_factor;
mod reset_password;

use crate::app::config::Config;

pub const HELP: &str = "\
Bloogla - a fast, single-binary blog engine

USAGE:
    bloogla [COMMAND]

COMMANDS:
    serve                     Start the web server (default)
    backup [FILE]             Save a copy of the database (default: data/backups/)
    reset-password [EMAIL]    Set a new admin password
    disable-2fa EMAIL         Turn off two-factor login for someone locked out
    help                      Show this message
    version                   Show the version

Configuration is read from BLOOGLA_* environment variables; see README.md.
";

/// What to do, from the command line arguments.
pub enum Command {
    Serve,
    Backup(Option<String>),
    ResetPassword(Option<String>),
    DisableTwoFactor(String),
    Help,
    Version,
}

impl Command {
    /// Read the command from the arguments (without the program name).
    /// Returns `None` for an unknown command.
    pub fn parse(args: &[String]) -> Option<Self> {
        let args: Vec<&str> = args.iter().map(String::as_str).collect();
        let command = match args.as_slice() {
            [] | ["serve"] => Command::Serve,
            ["backup"] => Command::Backup(None),
            ["backup", file] => Command::Backup(Some(file.to_string())),
            ["reset-password"] => Command::ResetPassword(None),
            ["reset-password", email] => Command::ResetPassword(Some(email.to_string())),
            ["disable-2fa", email] => Command::DisableTwoFactor(email.to_string()),
            ["help" | "--help" | "-h"] => Command::Help,
            ["version" | "--version" | "-V"] => Command::Version,
            _ => return None,
        };
        Some(command)
    }

    /// Run the command.
    pub async fn run(self) -> Result<(), Box<dyn std::error::Error>> {
        match self {
            Command::Help => {
                print!("{HELP}");
                return Ok(());
            }
            Command::Version => {
                println!("bloogla {}", env!("CARGO_PKG_VERSION"));
                return Ok(());
            }
            _ => {}
        }

        // Every other command works on the database in `data/`.
        let config = Config::from_env()?;
        std::fs::create_dir_all("data")?;
        crate::app::secrets::init()?;
        let pool = crate::db::connect(&config.database_url).await?;

        match self {
            Command::Serve => crate::server::run(config, pool).await,
            Command::Backup(file) => backup::run(&pool, file).await,
            Command::ResetPassword(email) => reset_password::run(&pool, email).await,
            Command::DisableTwoFactor(email) => disable_two_factor::run(&pool, &email).await,
            // Already handled above.
            Command::Help | Command::Version => Ok(()),
        }
    }
}
