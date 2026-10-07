//! Passwords, random tokens and API token hashing.

use argon2::password_hash::{PasswordHash, PasswordHasher, PasswordVerifier, SaltString};
use argon2::Argon2;
use rand::{Rng, RngCore};
use sha2::{Digest, Sha256};

/// Shortest password allowed, in characters.
pub const MIN_PASSWORD_LEN: usize = 12;

/// Something every new password must have.
///
/// The same rules are checked in the browser as people type, by
/// `admin/static/js/password.js`; keep the two in step (a test checks it).
pub struct PasswordRule {
    /// Short name the browser checklist uses, e.g. `upper`.
    pub id: &'static str,
    /// What to show people (English; translated in `src/i18n/`).
    pub text: &'static str,
    check: fn(&str) -> bool,
}

/// The rules for new passwords, in the order they're shown.
pub const PASSWORD_RULES: &[PasswordRule] = &[
    PasswordRule {
        id: "length",
        text: "At least 12 characters",
        check: long_enough,
    },
    PasswordRule {
        id: "lower",
        text: "A lowercase letter",
        check: has_lowercase,
    },
    PasswordRule {
        id: "upper",
        text: "An uppercase letter",
        check: has_uppercase,
    },
    PasswordRule {
        id: "number",
        text: "A number",
        check: has_number,
    },
    PasswordRule {
        id: "symbol",
        text: "A symbol, like ! or #",
        check: has_symbol,
    },
];

fn long_enough(password: &str) -> bool {
    password.chars().count() >= MIN_PASSWORD_LEN
}

fn has_lowercase(password: &str) -> bool {
    password.chars().any(char::is_lowercase)
}

fn has_uppercase(password: &str) -> bool {
    password.chars().any(char::is_uppercase)
}

fn has_number(password: &str) -> bool {
    password.chars().any(char::is_numeric)
}

/// Anything that isn't a letter, number or space: `!`, `#`, `€`, `-`...
fn has_symbol(password: &str) -> bool {
    password
        .chars()
        .any(|c| !c.is_alphanumeric() && !c.is_whitespace())
}

/// The rules a new password doesn't meet yet (empty when it's fine).
pub fn missing_password_rules(password: &str) -> Vec<&'static PasswordRule> {
    PASSWORD_RULES
        .iter()
        .filter(|rule| !(rule.check)(password))
        .collect()
}

/// Write a security event to the log: sign-ins, failed sign-ins, password
/// and role changes, API tokens. Every line starts with `security event=`, so
/// `journalctl -u bloogla | grep "security event"` lists them all.
///
/// Failures (`*_failed`, `*_rejected`, `*_throttled`) are warnings. Values are quoted, so
/// text people typed (like an email address) can't fake extra log lines.
pub fn log_event(event: &str, details: &[(&str, &str)]) {
    let details: String = details
        .iter()
        .map(|(key, value)| format!(" {key}={value:?}"))
        .collect();
    if ["_failed", "_rejected", "_throttled"]
        .iter()
        .any(|suffix| event.ends_with(suffix))
    {
        tracing::warn!("security event={event}{details}");
    } else {
        tracing::info!("security event={event}{details}");
    }
}

/// Hash a password with Argon2id for storing in the database.
pub fn hash_password(password: &str) -> Result<String, String> {
    let salt = SaltString::generate(&mut rand::thread_rng());
    Argon2::default()
        .hash_password(password.as_bytes(), &salt)
        .map(|h| h.to_string())
        .map_err(|e| format!("Password hashing failed: {e}"))
}

/// Check a password against a stored hash.
///
/// With no stored hash (unknown account) a dummy hash is checked anyway, so
/// the response time doesn't reveal which email addresses have accounts.
pub fn verify_password(password: &str, stored_hash: Option<&str>) -> bool {
    const DUMMY_HASH: &str = "$argon2id$v=19$m=19456,t=2,p=1$c29tZXNhbHRzb21lc2FsdA$J6ycAUHrfWLDxBqrLOjqEexZqnyJhGFRQnwxe4XiExI";
    let Ok(hash) = PasswordHash::new(stored_hash.unwrap_or(DUMMY_HASH)) else {
        return false;
    };
    let matches = Argon2::default()
        .verify_password(password.as_bytes(), &hash)
        .is_ok();
    matches && stored_hash.is_some()
}

/// Bytes as lowercase hex, two characters per byte.
pub fn to_hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

/// Hex back to bytes; `None` if it isn't valid hex.
pub fn from_hex(hex: &str) -> Option<Vec<u8>> {
    if !hex.len().is_multiple_of(2) {
        return None;
    }
    (0..hex.len())
        .step_by(2)
        .map(|i| u8::from_str_radix(hex.get(i..i + 2)?, 16).ok())
        .collect()
}

/// `bytes` random bytes as lowercase hex (twice as many characters).
pub fn random_hex(bytes: usize) -> String {
    let mut buffer = vec![0u8; bytes];
    rand::thread_rng().fill_bytes(&mut buffer);
    to_hex(&buffer)
}

/// A readable temporary password that meets [`PASSWORD_RULES`]: no
/// look-alike characters (0/O, 1/l/I) and only easy-to-type symbols.
pub fn temporary_password() -> String {
    const CHARS: &[u8] = b"abcdefghjkmnpqrstuvwxyzABCDEFGHJKLMNPQRSTUVWXYZ23456789!#%+=?@";
    let mut rng = rand::thread_rng();
    loop {
        let password: String = (0..14)
            .map(|_| CHARS[rng.gen_range(0..CHARS.len())] as char)
            .collect();
        // Random picks occasionally miss a kind of character; just try again.
        if missing_password_rules(&password).is_empty() {
            return password;
        }
    }
}

/// Ten one-time recovery codes for two-factor login, like `k7m2p-9xq4t`.
/// Only their [`hash_recovery_code`] is stored.
pub fn new_recovery_codes() -> Vec<String> {
    const CHARS: &[u8] = b"abcdefghjkmnpqrstuvwxyz23456789";
    let mut rng = rand::thread_rng();
    let mut part = || -> String {
        (0..5)
            .map(|_| CHARS[rng.gen_range(0..CHARS.len())] as char)
            .collect()
    };
    (0..10).map(|_| format!("{}-{}", part(), part())).collect()
}

/// SHA-256 of a recovery code, ignoring case, spaces and dashes as typed.
pub fn hash_recovery_code(code: &str) -> String {
    let normalized: String = code
        .chars()
        .filter(|c| c.is_ascii_alphanumeric())
        .map(|c| c.to_ascii_lowercase())
        .collect();
    hash_token(&normalized)
}

/// A new API token. Only its [`hash_token`] is stored.
pub fn new_api_token() -> String {
    format!("bl_{}", random_hex(24))
}

/// SHA-256 of an API token, as stored in the database. Tokens are long and
/// random, so a fast hash is enough (unlike passwords).
pub fn hash_token(token: &str) -> String {
    to_hex(&Sha256::digest(token.as_bytes()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn passwords_round_trip() {
        let hash = hash_password("correct horse").unwrap();
        assert!(verify_password("correct horse", Some(&hash)));
        assert!(!verify_password("wrong horse", Some(&hash)));
        assert!(!verify_password("correct horse", None));
    }

    #[test]
    fn password_rules_say_what_is_missing() {
        let missing =
            |p: &str| -> Vec<&str> { missing_password_rules(p).iter().map(|r| r.id).collect() };
        assert_eq!(
            missing(""),
            ["length", "lower", "upper", "number", "symbol"]
        );
        assert_eq!(missing("password"), ["length", "upper", "number", "symbol"]);
        assert_eq!(missing("passwordpassword"), ["upper", "number", "symbol"]);
        assert_eq!(missing("Password1234"), ["symbol"]);
        assert_eq!(missing("Pass word1!"), ["length"]);
        assert!(missing("Long pass word1!").is_empty());
        // The shown text must state the real minimum.
        assert!(PASSWORD_RULES[0]
            .text
            .contains(&MIN_PASSWORD_LEN.to_string()));
        // Non-English letters and symbols count too.
        assert!(missing("Güçlü şifrem2026€").is_empty());
    }

    #[test]
    fn temporary_passwords_meet_the_rules() {
        for _ in 0..200 {
            assert!(missing_password_rules(&temporary_password()).is_empty());
        }
    }

    #[test]
    fn browser_checklist_matches_the_rules() {
        let script = include_str!("../../admin/static/js/password.js");
        assert!(
            script.contains(&format!("const MIN_LENGTH = {MIN_PASSWORD_LEN};")),
            "password.js MIN_LENGTH differs from MIN_PASSWORD_LEN"
        );
        for rule in PASSWORD_RULES {
            assert!(
                script.contains(&format!("{}:", rule.id)),
                "password.js has no check for `{}`",
                rule.id
            );
        }
    }

    #[test]
    fn recovery_codes_are_readable_and_forgiving() {
        let codes = new_recovery_codes();
        assert_eq!(codes.len(), 10);
        assert!(codes
            .iter()
            .all(|c| c.len() == 11 && c.as_bytes()[5] == b'-'));
        assert_eq!(
            hash_recovery_code("K7M2P 9XQ4T"),
            hash_recovery_code("k7m2p-9xq4t")
        );
    }

    #[test]
    fn tokens_are_random_and_prefixed() {
        let (a, b) = (new_api_token(), new_api_token());
        assert!(a.starts_with("bl_") && a.len() == 51);
        assert_ne!(a, b);
    }
}
