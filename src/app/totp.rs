//! Two-factor login codes (TOTP, RFC 6238): the 6-digit codes that
//! authenticator apps such as Google Authenticator, 1Password or Authy show.
//!
//! The app and Bloogla share a secret key. Every 30 seconds both compute a
//! code from the key and the current time; if they match, the person has the
//! phone (or app) that holds the key.

use hmac::{Hmac, Mac};
use sha1::Sha1;

/// Seconds each code is valid for.
const STEP_SECONDS: u64 = 30;
const DIGITS: u32 = 6;
/// Codes from one step before or after are accepted too (clock drift).
const ALLOWED_DRIFT: i64 = 1;

/// A new random secret key (160 bits, as authenticator apps expect).
pub fn new_secret() -> Vec<u8> {
    use rand::RngCore;
    let mut key = vec![0u8; 20];
    rand::thread_rng().fill_bytes(&mut key);
    key
}

/// The code for one 30-second time step.
fn code_at(secret: &[u8], step: u64) -> u32 {
    let mut mac = Hmac::<Sha1>::new_from_slice(secret).expect("HMAC accepts any key length");
    mac.update(&step.to_be_bytes());
    let hash = mac.finalize().into_bytes();
    // "Dynamic truncation" from RFC 4226.
    let offset = (hash[19] & 0x0f) as usize;
    let number = u32::from_be_bytes([
        hash[offset],
        hash[offset + 1],
        hash[offset + 2],
        hash[offset + 3],
    ]) & 0x7fff_ffff;
    number % 10u32.pow(DIGITS)
}

/// Check a typed code against the current time (`unix_seconds`).
///
/// Returns the time step it matched, so the caller can refuse to accept that
/// step (or an earlier one) again. Spaces in the typed code are ignored.
pub fn verify(secret: &[u8], typed: &str, unix_seconds: u64, last_used_step: u64) -> Option<u64> {
    let typed: String = typed.chars().filter(|c| !c.is_whitespace()).collect();
    if typed.len() != DIGITS as usize || !typed.chars().all(|c| c.is_ascii_digit()) {
        return None;
    }
    let typed: u32 = typed.parse().ok()?;
    let now = (unix_seconds / STEP_SECONDS) as i64;
    (-ALLOWED_DRIFT..=ALLOWED_DRIFT)
        .map(|drift| (now + drift) as u64)
        .filter(|step| *step > last_used_step)
        .find(|step| code_at(secret, *step) == typed)
}

/// The secret as authenticator apps show it: Base32, in groups of four.
pub fn display_secret(secret: &[u8]) -> String {
    let base32 = base32(secret);
    base32
        .as_bytes()
        .chunks(4)
        .map(|c| String::from_utf8_lossy(c).into_owned())
        .collect::<Vec<_>>()
        .join(" ")
}

/// The `otpauth://` link authenticator apps read from the QR code.
pub fn setup_uri(secret: &[u8], site_name: &str, email: &str) -> String {
    let issuer = crate::content::text::url_encode(site_name);
    let account = crate::content::text::url_encode(email);
    format!(
        "otpauth://totp/{issuer}:{account}?secret={}&issuer={issuer}&algorithm=SHA1&digits={DIGITS}&period={STEP_SECONDS}",
        base32(secret)
    )
}

/// The setup link as a QR code (SVG) for the app to scan.
pub fn qr_svg(uri: &str) -> String {
    match qrcode::QrCode::new(uri.as_bytes()) {
        Ok(code) => code
            .render::<qrcode::render::svg::Color>()
            .min_dimensions(200, 200)
            .quiet_zone(true)
            .build(),
        Err(e) => {
            tracing::error!("Could not make QR code: {e}");
            String::new()
        }
    }
}

/// RFC 4648 Base32 without padding.
fn base32(bytes: &[u8]) -> String {
    const ALPHABET: &[u8; 32] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZ234567";
    let mut out = String::new();
    let (mut buffer, mut bits) = (0u32, 0u32);
    for &byte in bytes {
        buffer = (buffer << 8) | byte as u32;
        bits += 8;
        while bits >= 5 {
            out.push(ALPHABET[((buffer >> (bits - 5)) & 31) as usize] as char);
            bits -= 5;
        }
    }
    if bits > 0 {
        out.push(ALPHABET[((buffer << (5 - bits)) & 31) as usize] as char);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    const RFC_SECRET: &[u8] = b"12345678901234567890";

    #[test]
    fn matches_the_rfc_6238_test_vectors() {
        // RFC 6238 appendix B lists 8-digit codes; the last 6 digits are ours.
        for (time, eight_digits) in [
            (59u64, "94287082"),
            (1111111109, "07081804"),
            (1234567890, "89005924"),
            (2000000000, "69279037"),
        ] {
            assert_eq!(
                format!("{:06}", code_at(RFC_SECRET, time / 30)),
                &eight_digits[2..]
            );
        }
    }

    #[test]
    fn accepts_each_code_once_and_allows_small_drift() {
        let now = 1_234_567_890;
        let code = format!("{:06}", code_at(RFC_SECRET, now / 30));
        let step = verify(RFC_SECRET, &code, now, 0).expect("current code works");
        assert_eq!(
            verify(RFC_SECRET, &code, now, step),
            None,
            "same code twice"
        );
        assert!(
            verify(RFC_SECRET, &code, now + 30, 0).is_some(),
            "30 s late still fine"
        );
        assert!(verify(RFC_SECRET, &code, now + 90, 0).is_none(), "too late");
        assert!(verify(RFC_SECRET, "12 34 5", now, 0).is_none());
        let spaced = format!("{} {}", &code[..3], &code[3..]);
        assert!(
            verify(RFC_SECRET, &spaced, now, 0).is_some(),
            "spaces are ignored"
        );
    }

    #[test]
    fn base32_matches_rfc_4648() {
        assert_eq!(base32(b"foobar"), "MZXW6YTBOI");
        assert_eq!(base32(b"f"), "MY");
        assert!(setup_uri(RFC_SECRET, "My Blog", "a@b.c").starts_with("otpauth://totp/My%20Blog:a%40b.c?secret=GEZDGNBVGY3TQOJQGEZDGNBVGY3TQOJQ&issuer=My%20Blog"));
        assert!(qr_svg("otpauth://totp/x").starts_with("<?xml"));
    }
}
