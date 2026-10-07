//! Encrypting secrets that Bloogla stores in the database: the mail server
//! password and two-factor login keys.
//!
//! The key lives outside the database, in `data/secret.key` (created on first
//! start, readable only by Bloogla's user). A copy of the database alone, such
//! as a leaked backup, doesn't reveal the secrets.
//!
//! Keep `data/secret.key` together with your backups: without it, the mail
//! server password has to be entered again and two-factor login set up again.

use crate::app::security::{from_hex, to_hex};
use chacha20poly1305::aead::{Aead, AeadCore, KeyInit, OsRng};
use chacha20poly1305::{ChaCha20Poly1305, Key, Nonce};

use std::path::Path;
use std::sync::OnceLock;

const KEY_FILE: &str = "data/secret.key";
/// Marks an encrypted value, so plain values written earlier still read.
const PREFIX: &str = "enc:v1:";

static CIPHER: OnceLock<ChaCha20Poly1305> = OnceLock::new();

/// Load the key from `data/secret.key`, creating it the first time.
pub fn init() -> Result<(), String> {
    let path = Path::new(KEY_FILE);
    let key = if path.exists() {
        let hex =
            std::fs::read_to_string(path).map_err(|e| format!("Could not read {KEY_FILE}: {e}"))?;
        from_hex(hex.trim())
            .filter(|k| k.len() == 32)
            .ok_or(format!("{KEY_FILE} is damaged"))?
    } else {
        let key = ChaCha20Poly1305::generate_key(&mut OsRng).to_vec();
        write_private(path, &to_hex(&key))
            .map_err(|e| format!("Could not create {KEY_FILE}: {e}"))?;
        tracing::info!("Created {KEY_FILE}; keep it with your backups");
        key
    };
    use_key(&key);
    Ok(())
}

fn use_key(key: &[u8]) {
    let _ = CIPHER.set(ChaCha20Poly1305::new(Key::from_slice(key)));
}

/// Write a file only its owner can read.
fn write_private(path: &Path, contents: &str) -> std::io::Result<()> {
    use std::io::Write;
    let mut options = std::fs::OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    std::os::unix::fs::OpenOptionsExt::mode(&mut options, 0o600);
    options.open(path)?.write_all(contents.as_bytes())
}

/// Encrypt a secret for storing. Empty stays empty.
pub fn encrypt(plain: &str) -> String {
    if plain.is_empty() {
        return String::new();
    }
    let cipher = CIPHER.get().expect("secrets::init() runs at start");
    let nonce = ChaCha20Poly1305::generate_nonce(&mut OsRng);
    let sealed = cipher
        .encrypt(&nonce, plain.as_bytes())
        .expect("encrypting in memory can't fail");
    format!("{PREFIX}{}{}", to_hex(&nonce), to_hex(&sealed))
}

/// Read a stored secret. `None` when it can't be decrypted (for example the
/// database was moved without its `data/secret.key`).
pub fn decrypt(stored: &str) -> Option<String> {
    let Some(hex) = stored.strip_prefix(PREFIX) else {
        // Written before encryption existed.
        return Some(stored.to_string());
    };
    let bytes = from_hex(hex)?;
    if bytes.len() < 12 {
        return None;
    }
    let (nonce, sealed) = bytes.split_at(12);
    let plain = CIPHER
        .get()?
        .decrypt(Nonce::from_slice(nonce), sealed)
        .ok()?;
    String::from_utf8(plain).ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn secrets_round_trip_and_detect_tampering() {
        use_key(&[7u8; 32]);
        let stored = encrypt("hunter2");
        assert!(stored.starts_with(PREFIX) && !stored.contains("hunter2"));
        assert_ne!(
            stored,
            encrypt("hunter2"),
            "each encryption uses a new nonce"
        );
        assert_eq!(decrypt(&stored).as_deref(), Some("hunter2"));
        // Changing one character breaks the authentication tag.
        let mut tampered = stored.clone();
        let last = tampered.pop().unwrap();
        tampered.push(if last == '0' { '1' } else { '0' });
        assert_eq!(decrypt(&tampered), None);
        // Values from before encryption still read; empty stays empty.
        assert_eq!(decrypt("plain").as_deref(), Some("plain"));
        assert_eq!(encrypt(""), "");
    }
}
