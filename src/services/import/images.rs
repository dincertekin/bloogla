//! The images of an imported site: finding them in posts, downloading them
//! from the old site, and pointing the posts at their copies in Media.
//!
//! Downloading an address from a file someone uploaded is risky: the file
//! could point at this server itself or at other machines on its network.
//! So only public internet addresses are used (checked every time Bloogla
//! connects, also after a redirect), with limits on time, size and redirects,
//! and only real images are kept (Media checks every file's content).

use std::io::ErrorKind;
use std::net::IpAddr;
use std::time::Duration;
use ureq::unversioned::resolver::{DefaultResolver, ResolvedSocketAddrs, Resolver};

/// Why an image wasn't copied (English; the report translates it).
pub const NOT_FOUND: &str = "the old site said it doesn't exist";
pub const TOO_SLOW: &str = "took too long";
pub const TOO_LARGE: &str = "too large";
pub const PRIVATE: &str = "it's on a private network, so it wasn't copied";
pub const UNREACHABLE: &str = "the old site couldn't be reached";
pub const REFUSED: &str = "the old site didn't send it";
pub const OTHER_SITE: &str = "it's on another site, so it wasn't copied";
pub const NOT_IMAGE: &str = "it isn't a JPEG, PNG, GIF or WebP image";

/// File types copied; Media checks the content again.
const IMAGE_ENDINGS: &[&str] = &[".jpg", ".jpeg", ".png", ".gif", ".webp"];

/// The images on the old site that `html` shows, in order, each once.
/// Only full addresses (`https://...`) count; WordPress always writes those.
pub fn find(html: &str) -> Vec<String> {
    let mut found: Vec<String> = Vec::new();
    let mut rest = html;
    while let Some(start) = rest.find("http") {
        let candidate = &rest[start..];
        // An address ends at a quote, a space, or the end of a tag.
        let end = candidate
            .find(|c: char| c.is_whitespace() || matches!(c, '"' | '\'' | '<' | '>' | ')' | ','))
            .unwrap_or(candidate.len());
        let url = &candidate[..end];
        let is_web = url.starts_with("http://") || url.starts_with("https://");
        if is_web && is_image(url) && !found.iter().any(|f| f == url) {
            found.push(url.to_string());
        }
        rest = &candidate[end.max(4)..];
    }
    found
}

/// Whether an address names an image file (by its ending).
fn is_image(url: &str) -> bool {
    let path = url.split(['?', '#']).next().unwrap_or(url).to_lowercase();
    IMAGE_ENDINGS.iter().any(|ending| path.ends_with(ending))
}

/// The site an address is on, without `www.`: `ayseskitchen.com`.
pub fn site_of(url: &str) -> Option<String> {
    let after_scheme = url.split_once("://")?.1;
    let host = after_scheme.split(['/', '?', '#']).next()?;
    let host = host.rsplit_once('@').map_or(host, |(_, h)| h);
    let host = host.split(':').next()?.to_lowercase();
    let host = host.strip_prefix("www.").unwrap_or(&host).to_string();
    (!host.is_empty()).then_some(host)
}

/// The full-size image behind a smaller copy WordPress made:
/// `cake-1024x768.jpg` → `cake.jpg`. Other addresses stay as they are.
/// (A `?version` after it is dropped too: it names the same file.)
pub fn full_size(url: &str) -> String {
    let path = url.split(['?', '#']).next().unwrap_or(url);
    let Some(dot) = path.rfind('.') else {
        return path.to_string();
    };
    let (name, ending) = path.split_at(dot);
    let digits = |s: &str| !s.is_empty() && s.chars().all(|c| c.is_ascii_digit());
    if let Some((base, size)) = name.rsplit_once('-') {
        let is_size = size
            .split_once('x')
            .is_some_and(|(w, h)| digits(w) && digits(h));
        if is_size && !base.ends_with('/') {
            return format!("{base}{ending}");
        }
    }
    path.to_string()
}

/// Give `html` with every address in `replace` changed to its new one, and
/// without `srcset`/`sizes`, which list WordPress's own smaller copies (Bloogla
/// makes its own).
pub fn point_to_copies(html: &str, replace: &[(String, String)]) -> String {
    let mut out = without_attribute(&without_attribute(html, "srcset"), "sizes");
    // Longest first, so `cake-300x200.jpg` isn't half-replaced by `cake.jpg`.
    let mut replace = replace.to_vec();
    replace.sort_by_key(|(old, _)| std::cmp::Reverse(old.len()));
    for (old, new) in &replace {
        out = out.replace(old.as_str(), new);
    }
    out
}

/// `html` without any ` name="..."` attribute.
fn without_attribute(html: &str, name: &str) -> String {
    let mut out = String::with_capacity(html.len());
    let mut rest = html;
    let needle = format!(" {name}=\"");
    while let Some(start) = rest.find(&needle) {
        let value_start = start + needle.len();
        let Some(length) = rest[value_start..].find('"') else {
            break;
        };
        out.push_str(&rest[..start]);
        rest = &rest[value_start + length + 1..];
    }
    out.push_str(rest);
    out
}

/// Download one image. Blocking: run it with `spawn_blocking`.
pub fn download(url: &str, max_bytes: u64) -> Result<Vec<u8>, &'static str> {
    let response = agent().get(url).call();
    let mut response = match response {
        Ok(response) => response,
        Err(error) => return Err(reason(&error)),
    };
    response
        .body_mut()
        .with_config()
        .limit(max_bytes)
        .read_to_vec()
        .map_err(|error| reason(&error))
}

/// The plain reason for a failed download.
fn reason(error: &ureq::Error) -> &'static str {
    match error {
        ureq::Error::StatusCode(404 | 410) => NOT_FOUND,
        ureq::Error::StatusCode(_) => REFUSED,
        ureq::Error::Timeout(_) => TOO_SLOW,
        ureq::Error::BodyExceedsLimit(_) => TOO_LARGE,
        ureq::Error::Io(e) if e.kind() == ErrorKind::PermissionDenied => PRIVATE,
        _ => UNREACHABLE,
    }
}

/// The downloader: public addresses only, 30 seconds per image, at most
/// three redirects.
fn agent() -> ureq::Agent {
    let config = ureq::Agent::config_builder()
        .timeout_global(Some(Duration::from_secs(30)))
        .max_redirects(3)
        .user_agent(format!("Bloogla/{}", env!("CARGO_PKG_VERSION")))
        .build();
    ureq::Agent::with_parts(
        config,
        ureq::unversioned::transport::DefaultConnector::default(),
        PublicOnly::default(),
    )
}

/// Looks up a site's address like usual, but refuses addresses that aren't
/// on the public internet. Every connection goes through it, redirects too.
#[derive(Debug, Default)]
struct PublicOnly(DefaultResolver);

impl Resolver for PublicOnly {
    fn resolve(
        &self,
        uri: &ureq::http::Uri,
        config: &ureq::config::Config,
        timeout: ureq::unversioned::transport::NextTimeout,
    ) -> Result<ResolvedSocketAddrs, ureq::Error> {
        let all = self.0.resolve(uri, config, timeout)?;
        let mut public = self.empty();
        for address in all.iter().filter(|a| is_public(a.ip())) {
            public.push(*address);
        }
        if public.is_empty() {
            return Err(ureq::Error::Io(std::io::Error::new(
                ErrorKind::PermissionDenied,
                "not a public internet address",
            )));
        }
        Ok(public)
    }
}

/// Whether an address is on the public internet (not this machine, not a
/// home or office network, not reserved).
fn is_public(ip: IpAddr) -> bool {
    match ip {
        IpAddr::V4(v4) => {
            let [a, b, ..] = v4.octets();
            !(v4.is_private()
                || v4.is_loopback()
                || v4.is_link_local()
                || v4.is_broadcast()
                || v4.is_documentation()
                || v4.is_unspecified()
                || v4.is_multicast()
                || a == 0
                || (a == 100 && (64..128).contains(&b)) // shared by internet providers
                || (a == 192 && b == 0) // 192.0.0.x, protocol use
                || (a == 198 && (18..20).contains(&b)) // benchmarking
                || a >= 240) // reserved
        }
        IpAddr::V6(v6) => {
            if let Some(v4) = v6.to_ipv4_mapped() {
                return is_public(IpAddr::V4(v4));
            }
            let first = v6.segments()[0];
            !(v6.is_loopback()
                || v6.is_unspecified()
                || v6.is_multicast()
                || (first & 0xfe00) == 0xfc00 // unique local
                || (first & 0xffc0) == 0xfe80 // link-local
                || first == 0x2001 && v6.segments()[1] == 0x0db8) // documentation
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn finds_each_image_once() {
        let html = r#"<img src="https://a.com/up/cake-300x200.jpg" srcset="https://a.com/up/cake-300x200.jpg 300w, https://a.com/up/cake.jpg 1200w" sizes="(max-width: 300px) 100vw">
<a href="https://a.com/up/cake.jpg?x=1">full</a> <a href="https://a.com/menu.pdf">menu</a> http://b.org/x.PNG"#;
        assert_eq!(
            find(html),
            [
                "https://a.com/up/cake-300x200.jpg",
                "https://a.com/up/cake.jpg",
                "https://a.com/up/cake.jpg?x=1",
                "http://b.org/x.PNG",
            ]
        );
    }

    #[test]
    fn knows_the_site_and_the_full_size_image() {
        assert_eq!(
            site_of("https://www.Ayse.com:8080/a.jpg").as_deref(),
            Some("ayse.com")
        );
        assert_eq!(site_of("not an address"), None);
        assert_eq!(
            full_size("https://a.com/up/cake-1024x768.jpg"),
            "https://a.com/up/cake.jpg"
        );
        assert_eq!(
            full_size("https://a.com/up/cake-1024x768.jpg?v=2"),
            "https://a.com/up/cake.jpg"
        );
        assert_eq!(
            full_size("https://a.com/up/my-cake.jpg"),
            "https://a.com/up/my-cake.jpg"
        );
        assert_eq!(
            full_size("https://a.com/up/cake-x.jpg"),
            "https://a.com/up/cake-x.jpg"
        );
    }

    #[test]
    fn points_posts_at_the_copies() {
        let html = r#"<img src="https://a.com/up/cake-300x200.jpg" srcset="https://a.com/up/cake-300x200.jpg 300w" sizes="100vw" alt="Cake">"#;
        let replace = vec![
            (
                "https://a.com/up/cake.jpg".to_string(),
                "/uploads/new.jpg".to_string(),
            ),
            (
                "https://a.com/up/cake-300x200.jpg".to_string(),
                "/uploads/new.jpg".to_string(),
            ),
        ];
        assert_eq!(
            point_to_copies(html, &replace),
            r#"<img src="/uploads/new.jpg" alt="Cake">"#
        );
    }

    #[test]
    fn only_public_internet_addresses_are_used() {
        for private in [
            "127.0.0.1",
            "10.1.2.3",
            "172.16.0.9",
            "192.168.1.20",
            "169.254.169.254",
            "100.64.0.1",
            "0.0.0.0",
            "::1",
            "fc00::1",
            "fe80::1",
            "::ffff:192.168.1.1",
        ] {
            assert!(!is_public(private.parse().unwrap()), "{private}");
        }
        for public in ["93.184.216.34", "1.1.1.1", "2606:4700:4700::1111"] {
            assert!(is_public(public.parse().unwrap()), "{public}");
        }
    }

    #[test]
    fn this_machine_is_never_downloaded_from() {
        // Refused before connecting, so this needs no server.
        assert_eq!(download("http://127.0.0.1:9/cake.jpg", 1000), Err(PRIVATE));
        assert_eq!(download("http://localhost:9/cake.jpg", 1000), Err(PRIVATE));
    }
}
