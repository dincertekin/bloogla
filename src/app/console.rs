//! What a person sees in the terminal window Bloogla runs in.
//!
//! Started by a person (a terminal window, or a double-click), Bloogla shows
//! a calm start screen, short lines for things that matter ("A comment is
//! waiting"), and plain-language problems. Started by a server (Docker,
//! systemd, output going to a file), it writes ordinary log lines instead,
//! which hosting tools understand. [`for_a_person`] decides which.
//!
//! Everything here is translated into the site's language.

use crate::i18n::Lang;

use std::io::{IsTerminal, Write};
use std::path::Path;
use std::sync::OnceLock;

/// True when a person is watching: the output is a terminal window and
/// nobody asked for plain logs with `BLOOGLA_LOG` or `BLOOGLA_LOG_FORMAT`.
pub fn for_a_person() -> bool {
    static ANSWER: OnceLock<bool> = OnceLock::new();
    *ANSWER.get_or_init(|| {
        std::io::stderr().is_terminal()
            && std::env::var_os("BLOOGLA_LOG").is_none()
            && std::env::var_os("BLOOGLA_LOG_FORMAT").is_none()
    })
}

/// The site's language for console text (English until the site has one).
fn lang() -> Lang {
    crate::db::settings::cached_language().unwrap_or_default()
}

// ---- Looks ----

/// Colours, except where they'd show as garbage: older Windows consoles, and
/// people who turned them off (`NO_COLOR`, see no-color.org).
fn colors() -> bool {
    !cfg!(windows) && std::env::var_os("NO_COLOR").is_none()
}

fn paint(code: &str, text: &str) -> String {
    if colors() {
        format!("\x1b[{code}m{text}\x1b[0m")
    } else {
        text.to_string()
    }
}

fn bold(text: &str) -> String {
    paint("1", text)
}

fn dim(text: &str) -> String {
    paint("2", text)
}

/// What kind of news a line is, shown as a small mark in front of it.
#[derive(Clone, Copy)]
pub enum Kind {
    /// Something finished well: ✓
    Done,
    /// Worth knowing: •
    Note,
    /// Needs attention: !
    Problem,
}

impl Kind {
    fn mark(self) -> String {
        // Windows console fonts have √ but not always ✓.
        let check = if cfg!(windows) { "√" } else { "✓" };
        match self {
            Kind::Done => paint("32", check),
            Kind::Note => paint("2", "•"),
            Kind::Problem => paint("33", "!"),
        }
    }
}

// ---- What gets shown ----

/// One line of news while the site runs: `10:42  ✓ Daily backup saved.`
/// Servers get an ordinary log line instead.
pub fn activity(kind: Kind, text: &str) {
    if !for_a_person() {
        match kind {
            Kind::Problem => tracing::warn!("{text}"),
            _ => tracing::info!("{text}"),
        }
        return;
    }
    let time = chrono::Local::now().format("%H:%M").to_string();
    eprintln!("  {}  {} {text}", dim(&time), kind.mark());
}

/// Translated activity lines, for code that doesn't have a language at hand.
pub fn activity_t(kind: Kind, english: &'static str) {
    activity(kind, lang().t(english));
}

/// Like [`activity_t`] with one `{placeholder}`.
pub fn activity_tv(kind: Kind, english: &'static str, value: impl std::fmt::Display) {
    activity(kind, &lang().tv(english, value));
}

/// Everything someone needs once the site is up.
pub struct StartScreen<'a> {
    /// The site's address, e.g. `http://localhost:8080`.
    pub site_url: &'a str,
    /// The one-time setup link, until the site is set up.
    pub setup_url: Option<String>,
    /// The folder with the database, pictures and themes.
    pub folder: &'a Path,
}

/// The start screen (servers get two log lines instead). On a fresh site on
/// a desktop computer, the setup page also opens in the browser.
pub fn start_screen(screen: StartScreen) {
    if !for_a_person() {
        tracing::info!("Your site is running at {}", screen.site_url);
        return;
    }
    let lang = lang();
    let opened = screen.setup_url.as_deref().is_some_and(open_in_browser);

    // Labels padded to one width, so the values line up.
    let labels = [
        lang.t("Open your site"),
        lang.t("Finish setup"),
        lang.t("Your files"),
    ];
    let width = labels.iter().map(|l| l.chars().count()).max().unwrap_or(0) + 4;
    let row = |label: &str, value: &str| {
        let pad = " ".repeat(width - label.chars().count());
        format!("    {}{pad}{value}", dim(label))
    };
    let indent = " ".repeat(width + 4);

    let mut out = String::new();
    out.push_str(&format!(
        "\n  {}\n\n",
        bold(&format!("Bloogla {}", env!("CARGO_PKG_VERSION")))
    ));
    out.push_str(&format!(
        "  {} {}\n\n",
        Kind::Done.mark(),
        bold(lang.t("Your site is running"))
    ));
    out.push_str(&row(labels[0], screen.site_url));
    out.push('\n');
    if let Some(setup) = &screen.setup_url {
        out.push_str(&row(labels[1], &bold(setup)));
        out.push('\n');
        if opened {
            out.push_str(&format!(
                "{indent}{}\n",
                dim(lang.t("(opened in your browser)"))
            ));
        }
    }
    out.push('\n');
    out.push_str(&row(labels[2], &screen.folder.display().to_string()));
    out.push('\n');
    out.push_str(&format!(
        "{indent}{}\n\n",
        dim(lang.t("Posts, pictures and backups. Keep this folder safe."))
    ));
    out.push_str(&format!(
        "  {}\n  {}\n\n  {}\n",
        lang.t("Keep this window open while your site is online."),
        dim(lang.t("Press Ctrl+C to stop.")),
        dim(&"─".repeat(48)),
    ));
    let _ = std::io::stderr().write_all(out.as_bytes());
}

/// Said when Ctrl+C (or the system) asks Bloogla to stop.
pub fn stopping() {
    if for_a_person() {
        eprintln!("\n  {}", dim(lang().t("Stopping…")));
    } else {
        tracing::info!("Shutting down");
    }
}

/// Said once everything is saved and closed.
pub fn stopped() {
    if for_a_person() {
        eprintln!(
            "  {} {}\n",
            Kind::Done.mark(),
            lang().t("Bloogla stopped. Your site is offline until you start it again.")
        );
    } else {
        tracing::info!("Stopped");
    }
}

/// A problem that stops Bloogla, said plainly. On Windows the window would
/// close at once (it was opened by double-clicking), so it waits for Enter.
pub fn failed(message: &str) {
    if !for_a_person() {
        eprintln!("Error: {message}");
        return;
    }
    let lang = lang();
    eprintln!(
        "\n  {} {}\n\n    {message}\n",
        Kind::Problem.mark(),
        bold(lang.t("Bloogla couldn't continue"))
    );
    if cfg!(windows) {
        eprintln!("  {}", dim(lang.t("Press Enter to close this window.")));
        let _ = std::io::stdin().read_line(&mut String::new());
    }
}

/// Open `url` in the default browser on a desktop computer. Does nothing on
/// servers (no screen), so it's safe to call anywhere.
pub fn open_in_browser(url: &str) -> bool {
    use std::process::{Command, Stdio};
    let mut command = if cfg!(target_os = "macos") {
        Command::new("open")
    } else if cfg!(windows) {
        let mut start = Command::new("cmd");
        start.args(["/C", "start", ""]);
        start
    } else if std::env::var_os("DISPLAY").is_some() || std::env::var_os("WAYLAND_DISPLAY").is_some()
    {
        Command::new("xdg-open")
    } else {
        return false; // A server without a screen.
    };
    command
        .arg(url)
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .is_ok_and(|status| status.success())
}

/// Turn a failure to listen on a port into words a person understands.
pub fn port_problem(port: u16, error: &std::io::Error) -> String {
    let lang = lang();
    match error.kind() {
        std::io::ErrorKind::AddrInUse => lang.tv2(
            "Port {port} is already in use, probably by Bloogla running in another window. Close that one, or start Bloogla on another port with BLOOGLA_PORT={other}.",
            port,
            port.saturating_add(1),
        ),
        std::io::ErrorKind::PermissionDenied => lang.tv2(
            "Bloogla isn't allowed to use port {port}; ports below 1024 need administrator rights. Start it with BLOOGLA_PORT={other}, or as an administrator.",
            port,
            8080,
        ),
        _ => lang.tv2("Bloogla couldn't use port {port}: {error}", port, error),
    }
}

/// Log lines shown to a person: just the problems, as `10:42  ! message`.
/// (Ordinary progress is shown by [`activity`] instead.)
pub struct FriendlyLog;

impl<S, N> tracing_subscriber::fmt::FormatEvent<S, N> for FriendlyLog
where
    S: tracing::Subscriber + for<'a> tracing_subscriber::registry::LookupSpan<'a>,
    N: for<'a> tracing_subscriber::fmt::FormatFields<'a> + 'static,
{
    fn format_event(
        &self,
        ctx: &tracing_subscriber::fmt::FmtContext<'_, S, N>,
        mut writer: tracing_subscriber::fmt::format::Writer<'_>,
        event: &tracing::Event<'_>,
    ) -> std::fmt::Result {
        let time = chrono::Local::now().format("%H:%M").to_string();
        write!(writer, "  {}  {} ", dim(&time), Kind::Problem.mark())?;
        ctx.field_format().format_fields(writer.by_ref(), event)?;
        writeln!(writer)
    }
}
