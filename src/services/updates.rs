//! Updating Bloogla from GitHub, like WordPress's "update available".
//!
//! - **Settings → Updates** has "Check for updates", which asks GitHub for the
//!   latest release of github.com/dincertekin/bloogla, and two checkboxes
//!   (both off at first): check every day, and install new versions
//!   automatically. Nothing is checked when the server starts.
//! - **Installing** downloads the program for this server from the release,
//!   checks its signature (releases are signed with a private key only the
//!   GitHub repository has, see README "Releasing"), replaces the running
//!   program and restarts Bloogla in place.
//! - **Docker** sites can't replace their own program; they're shown the
//!   `docker pull` steps instead.
//!
//! GitHub only learns the server's address and Bloogla's version (User-Agent).

use crate::app::state::AppState;
use crate::db::settings;

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, OnceLock, RwLock};
use std::time::Duration;

/// Where Bloogla's releases are published.
pub const REPOSITORY: &str = "dincertekin/bloogla";

/// The version of this copy of Bloogla, from Cargo.toml.
pub const CURRENT_VERSION: &str = env!("CARGO_PKG_VERSION");

/// Public half of the release signing key (Ed25519). Downloads whose
/// signature doesn't match it are never installed.
const SIGNING_KEY: [u8; 32] = [
    0x63, 0x9d, 0x15, 0x20, 0xaf, 0xad, 0x8c, 0x91, 0x7c, 0xc0, 0x30, 0x07, 0xf2, 0x0c, 0x68, 0x81,
    0xa3, 0x03, 0x60, 0x25, 0x73, 0x2a, 0xc1, 0x7f, 0x14, 0x7b, 0xd0, 0x3a, 0x37, 0x34, 0x03, 0xdc,
];

/// The program for this kind of server in a release, next to `<name>.sig`
/// (see `.github/workflows/release.yml`). `None` where releases have none.
const ASSET: Option<&str> = if cfg!(all(target_os = "linux", target_arch = "x86_64")) {
    Some("bloogla-x86_64-linux")
} else if cfg!(all(target_os = "linux", target_arch = "aarch64")) {
    Some("bloogla-aarch64-linux")
} else {
    None
};

/// Largest program download accepted.
const MAX_DOWNLOAD: u64 = 100 * 1024 * 1024;

/// A published release.
#[derive(Debug, Clone, PartialEq)]
pub struct Release {
    /// e.g. `1.2.0` (without the `v` of the tag).
    pub version: String,
    /// Its page on GitHub, with the list of changes.
    pub url: String,
    /// The program for this server and its signature, if the release has them.
    pub download: Option<(String, String)>,
}

/// The newest release when it's newer than this copy, else `None`.
/// Kept in [`AppState`] so pages can read it without asking GitHub.
pub type NewerRelease = Arc<RwLock<Option<Release>>>;

/// The update a page should mention, if any.
pub fn available(newer: &NewerRelease) -> Option<Release> {
    newer.read().ok().and_then(|release| release.clone())
}

// ---- Checking ----

/// A check succeeded since Bloogla started (what it found is in AppState).
static CHECKED: AtomicBool = AtomicBool::new(false);

pub fn checked_since_start() -> bool {
    CHECKED.load(Ordering::Acquire)
}

/// Ask GitHub now, and remember the answer for the Dashboard and Settings.
pub async fn check(state: &AppState) -> Result<Option<Release>, String> {
    let latest = tokio::task::spawn_blocking(fetch_latest)
        .await
        .map_err(|e| e.to_string())??;
    CHECKED.store(true, Ordering::Release);
    let update = latest.filter(|release| is_newer(&release.version, CURRENT_VERSION));
    if let Ok(mut slot) = state.newer_release.write() {
        *slot = update.clone();
    }
    let now = chrono::Utc::now().format("%Y-%m-%d %H:%M:%S").to_string();
    if let Err(e) = settings::save(&state.pool, &[("update_last_checked", now)]).await {
        tracing::error!("Could not save when updates were checked: {e}");
    }
    Ok(update)
}

/// Every hour: when "Check for new versions every day" is on and a day has
/// passed, check; when "Install new versions automatically" is on too,
/// install what was found. The first look is an hour after starting.
pub fn spawn_daily_check(state: AppState) {
    tokio::spawn(async move {
        let hour = Duration::from_secs(60 * 60);
        let mut interval = tokio::time::interval_at(tokio::time::Instant::now() + hour, hour);
        loop {
            interval.tick().await;
            let site = settings::load(&state.pool).await;
            if !site.update_check_daily || !check_is_due(&site.update_last_checked) {
                continue;
            }
            match check(&state).await {
                Ok(Some(release)) => {
                    tracing::info!(
                        "Bloogla {} is available (this is {CURRENT_VERSION}): {}",
                        release.version,
                        release.url
                    );
                    if site.update_install_auto && install_method() == InstallMethod::Itself {
                        if let Err(e) = install(&release).await {
                            tracing::error!("Could not install Bloogla {}: {e}", release.version);
                        }
                    }
                }
                Ok(None) => {}
                Err(e) => tracing::warn!("Could not check for updates: {e}"),
            }
        }
    });
}

/// Whether a day has passed since `last_checked` (empty: never checked).
fn check_is_due(last_checked: &str) -> bool {
    match chrono::NaiveDateTime::parse_from_str(last_checked, "%Y-%m-%d %H:%M:%S") {
        Ok(last) => chrono::Utc::now().naive_utc() - last >= chrono::Duration::hours(24),
        Err(_) => true,
    }
}

/// The latest release on GitHub, or `None` if there isn't one yet.
fn fetch_latest() -> Result<Option<Release>, String> {
    let address = format!("https://api.github.com/repos/{REPOSITORY}/releases/latest");
    let mut response = match agent()
        .get(&address)
        .header("Accept", "application/vnd.github+json")
        .call()
    {
        Ok(response) => response,
        // No release published yet.
        Err(ureq::Error::StatusCode(404)) => return Ok(None),
        Err(e) => return Err(e.to_string()),
    };
    let body = response
        .body_mut()
        .read_to_string()
        .map_err(|e| e.to_string())?;
    parse_release(&body).map(Some)
}

/// HTTPS client with a time limit, telling GitHub who's asking.
fn agent() -> ureq::Agent {
    ureq::Agent::config_builder()
        .timeout_global(Some(Duration::from_secs(120)))
        .user_agent(format!("Bloogla/{CURRENT_VERSION}"))
        .build()
        .into()
}

/// Read the parts we need from GitHub's answer.
fn parse_release(body: &str) -> Result<Release, String> {
    let json: serde_json::Value = serde_json::from_str(body).map_err(|e| e.to_string())?;
    let tag = json["tag_name"].as_str().unwrap_or_default();
    let version = tag.trim_start_matches('v').to_string();
    if parse_version(&version).is_none() {
        return Err(format!("unexpected release name: {tag:?}"));
    }
    // Only ever link to, and download from, this repository on GitHub.
    let ours = |url: &str| url.starts_with(&format!("https://github.com/{REPOSITORY}/"));
    let url = match json["html_url"].as_str() {
        Some(url) if ours(url) => url.to_string(),
        _ => format!("https://github.com/{REPOSITORY}/releases"),
    };
    let asset_url = |name: &str| {
        json["assets"].as_array()?.iter().find_map(|asset| {
            let url = asset["browser_download_url"].as_str()?;
            (asset["name"].as_str() == Some(name) && ours(url)).then(|| url.to_string())
        })
    };
    let download =
        ASSET.and_then(|name| Some((asset_url(name)?, asset_url(&format!("{name}.sig"))?)));
    Ok(Release {
        version,
        url,
        download,
    })
}

/// `1.2.3` as numbers, so `1.10.0` counts as newer than `1.9.0`.
fn parse_version(version: &str) -> Option<(u64, u64, u64)> {
    let mut parts = version.split('.').map(|part| part.parse::<u64>().ok());
    let major = parts.next()??;
    let minor = parts.next().unwrap_or(Some(0))?;
    let patch = parts.next().unwrap_or(Some(0))?;
    if parts.next().is_some() {
        return None;
    }
    Some((major, minor, patch))
}

/// Whether `candidate` is a later version than `current`.
fn is_newer(candidate: &str, current: &str) -> bool {
    match (parse_version(candidate), parse_version(current)) {
        (Some(candidate), Some(current)) => candidate > current,
        _ => false,
    }
}

// ---- Installing ----

/// How this copy of Bloogla can be updated.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum InstallMethod {
    /// It can download the new version and restart itself.
    Itself,
    /// It runs in Docker: pull the new image instead.
    Docker,
    /// Replace the program by hand (another kind of server, or Bloogla
    /// isn't allowed to change its own file).
    ByHand,
}

pub fn install_method() -> InstallMethod {
    if Path::new("/.dockerenv").exists() || Path::new("/run/.containerenv").exists() {
        return InstallMethod::Docker;
    }
    if ASSET.is_none() || !may_replace_program() {
        return InstallMethod::ByHand;
    }
    InstallMethod::Itself
}

/// Whether Bloogla may write next to its own program file.
fn may_replace_program() -> bool {
    let Some(folder) = std::env::current_exe()
        .ok()
        .and_then(|exe| exe.parent().map(Path::to_path_buf))
    else {
        return false;
    };
    let probe = folder.join(".bloogla-update-check");
    let writable = std::fs::write(&probe, b"").is_ok();
    let _ = std::fs::remove_file(&probe);
    writable
}

/// True while an update is being downloaded, so it only happens once.
static INSTALLING: AtomicBool = AtomicBool::new(false);

/// Download the release, check its signature, put it in place of the
/// running program and restart. Nothing changes if any step fails.
pub async fn install(release: &Release) -> Result<(), String> {
    if INSTALLING.swap(true, Ordering::AcqRel) {
        return Err("An update is already being installed.".to_string());
    }
    let release = release.clone();
    let result = tokio::task::spawn_blocking(move || replace_program(&release))
        .await
        .map_err(|e| e.to_string())
        .and_then(|result| result);
    match result {
        Ok(program) => {
            tracing::info!("Installed a new version; restarting");
            restart_soon(program);
            Ok(())
        }
        Err(e) => {
            INSTALLING.store(false, Ordering::Release);
            Err(e)
        }
    }
}

/// Steps of [`install`] that wait for the network and disk. Returns the
/// program's path, to start it again.
fn replace_program(release: &Release) -> Result<PathBuf, String> {
    let (Some((program_url, signature_url)), Some(asset)) = (&release.download, ASSET) else {
        return Err("This release has no signed program for this server.".to_string());
    };
    let program = download(program_url, MAX_DOWNLOAD)?;
    let signature = download(signature_url, 1024)?;
    verify_signature(&release.version, asset, &program, &signature)?;

    // Write the new program next to the old one and make sure it starts and
    // is the version we expect, while the site is still running.
    let exe = std::env::current_exe().map_err(|e| e.to_string())?;
    let new = exe.with_extension("new");
    std::fs::write(&new, &program).map_err(|e| format!("Could not save the download: {e}"))?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&new, std::fs::Permissions::from_mode(0o755))
            .map_err(|e| e.to_string())?;
    }
    let answer = std::process::Command::new(&new)
        .arg("--version")
        .output()
        .map(|out| String::from_utf8_lossy(&out.stdout).trim().to_string())
        .unwrap_or_default();
    if answer != format!("bloogla {}", release.version) {
        let _ = std::fs::remove_file(&new);
        return Err("The downloaded program doesn't run on this server.".to_string());
    }

    // Swap them. The old one stays as `bloogla.old`, to go back by hand if
    // needed; if the swap fails halfway, the old one is put back.
    let old = exe.with_extension("old");
    std::fs::rename(&exe, &old).map_err(|e| e.to_string())?;
    if let Err(e) = std::fs::rename(&new, &exe) {
        let _ = std::fs::rename(&old, &exe);
        return Err(e.to_string());
    }
    Ok(exe)
}

/// What a release signature covers: the version and kind of server as well
/// as the program, so a signature can't be reused for another release (an
/// old version passed off as a new one). `.github/workflows/release.yml`
/// signs exactly this.
fn signed_message(version: &str, asset: &str, program: &[u8]) -> Vec<u8> {
    let mut message = format!("bloogla {version} {asset}\n").into_bytes();
    message.extend_from_slice(program);
    message
}

/// Check that Bloogla's release key signed this program for this release.
fn verify_signature(
    version: &str,
    asset: &str,
    program: &[u8],
    signature: &[u8],
) -> Result<(), String> {
    ring::signature::UnparsedPublicKey::new(&ring::signature::ED25519, SIGNING_KEY)
        .verify(&signed_message(version, asset, program), signature)
        .map_err(|_| "The download isn't signed with Bloogla's release key.".to_string())
}

/// The body of a download from this repository's releases.
fn download(url: &str, limit: u64) -> Result<Vec<u8>, String> {
    if !url.starts_with(&format!(
        "https://github.com/{REPOSITORY}/releases/download/"
    )) {
        return Err("Unexpected download address.".to_string());
    }
    agent()
        .get(url)
        .call()
        .map_err(|e| format!("Download failed: {e}"))?
        .body_mut()
        .with_config()
        .limit(limit)
        .read_to_vec()
        .map_err(|e| format!("Download failed: {e}"))
}

// ---- Restarting ----

/// The program to start again once the server has stopped (set after an update).
static RESTART_WITH: OnceLock<PathBuf> = OnceLock::new();

fn restart_signal() -> &'static tokio::sync::Notify {
    static SIGNAL: OnceLock<tokio::sync::Notify> = OnceLock::new();
    SIGNAL.get_or_init(tokio::sync::Notify::new)
}

/// Stop the server in a moment (after answering the admin who clicked
/// Install), then start the new program in its place.
fn restart_soon(program: PathBuf) {
    let _ = RESTART_WITH.set(program);
    tokio::spawn(async {
        tokio::time::sleep(Duration::from_secs(1)).await;
        restart_signal().notify_one();
    });
}

/// Resolves when an update asks the server to stop and restart.
pub async fn restart_requested() {
    restart_signal().notified().await;
}

/// After the server has stopped: if an update was installed, replace this
/// process with the new program (same arguments, same process id, so Docker,
/// systemd or a terminal keep running it). Returns if there's nothing to do.
pub fn restart_if_updated() -> Result<(), String> {
    let Some(program) = RESTART_WITH.get() else {
        return Ok(());
    };
    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt;
        let error = std::process::Command::new(program)
            .args(std::env::args_os().skip(1))
            .exec();
        Err(format!("Could not start the new version: {error}"))
    }
    #[cfg(not(unix))]
    {
        Err(format!(
            "Updated; start {} again to use the new version.",
            program.display()
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn versions_compare_as_numbers() {
        assert!(is_newer("1.0.1", "1.0.0"));
        assert!(is_newer("1.10.0", "1.9.0"));
        assert!(is_newer("2.0", "1.9.9"));
        assert!(!is_newer("1.0.0", "1.0.0"));
        assert!(!is_newer("0.9.0", "1.0.0"));
        assert!(
            !is_newer("1.1.0-beta", "1.0.0"),
            "test versions are skipped"
        );
        assert!(!is_newer("nonsense", "1.0.0"));
    }

    #[test]
    fn reads_githubs_answer() {
        let release = parse_release(
            r#"{"tag_name": "v1.2.0", "html_url": "https://github.com/dincertekin/bloogla/releases/tag/v1.2.0"}"#,
        )
        .unwrap();
        assert_eq!(release.version, "1.2.0");
        assert!(release.url.ends_with("/tag/v1.2.0"));
        assert_eq!(release.download, None, "no program attached");

        let elsewhere =
            parse_release(r#"{"tag_name": "v1.2.0", "html_url": "https://evil.example/"}"#)
                .unwrap();
        assert_eq!(
            elsewhere.url,
            "https://github.com/dincertekin/bloogla/releases"
        );

        assert!(parse_release(r#"{"tag_name": "latest"}"#).is_err());
    }

    #[test]
    fn a_daily_check_waits_a_day() {
        assert!(check_is_due(""));
        assert!(check_is_due("2020-01-01 00:00:00"));
        let now = chrono::Utc::now().format("%Y-%m-%d %H:%M:%S").to_string();
        assert!(!check_is_due(&now));
    }

    #[test]
    fn only_signed_programs_pass() {
        assert!(
            verify_signature("1.0.0", "bloogla-x86_64-linux", b"not bloogla", &[0u8; 64]).is_err()
        );
        assert_eq!(
            signed_message("1.2.0", "bloogla-x86_64-linux", b"PROGRAM"),
            b"bloogla 1.2.0 bloogla-x86_64-linux\nPROGRAM"
        );
    }
}
