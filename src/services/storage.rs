//! Copying the backup to storage somewhere else (Settings → Backup), so a
//! dead disk or a deleted server doesn't take the backups with it.
//!
//! Works with any storage that speaks Amazon's S3 language: Backblaze B2,
//! Cloudflare R2, Amazon S3, Wasabi, MinIO... Off until the owner turns it on.
//!
//! Once a day the whole site (the same .zip as "Download backup") is sent as
//! `bloogla-monday.zip`, `bloogla-tuesday.zip` and so on, so the storage
//! always holds the last 7 days without Bloogla having to delete anything.
//!
//! Requests are signed the way S3 expects ("Signature Version 4"), with the
//! `hmac` and `sha2` crates Bloogla already uses for other things.

use crate::app::console::{self, Kind};
use crate::app::security::to_hex;
use crate::db::settings::{self, Settings};

use hmac::{Hmac, KeyInit, Mac};
use sha2::{Digest, Sha256};
use sqlx::SqlitePool;
use std::sync::Mutex;
use std::time::{Duration, Instant};

/// Where to send backups: what the owner typed in Settings → Backup.
pub struct Storage {
    /// `https://s3.eu-central-003.backblazeb2.com`, without a trailing `/`.
    pub endpoint: String,
    pub region: String,
    pub bucket: String,
    pub access_key: String,
    pub secret_key: String,
}

impl Storage {
    /// The storage from the settings, when copying is on and every field is filled in.
    pub fn from_settings(site: &Settings) -> Option<Self> {
        let storage = Storage {
            endpoint: site.storage_endpoint.trim_end_matches('/').to_string(),
            region: site.storage_region.clone(),
            bucket: site.storage_bucket.clone(),
            access_key: site.storage_access_key.clone(),
            secret_key: site.storage_secret_key.clone(),
        };
        let complete = [
            &storage.endpoint,
            &storage.region,
            &storage.bucket,
            &storage.access_key,
            &storage.secret_key,
        ]
        .iter()
        .all(|field| !field.is_empty());
        (site.storage_on && complete).then_some(storage)
    }
}

/// How long one upload may take. A big site on a slow line needs a while.
const UPLOAD_TIME_LIMIT: Duration = Duration::from_secs(60 * 60);
const DAY: Duration = Duration::from_secs(24 * 60 * 60);

/// When the last daily copy was tried, so a failing one isn't retried every hour.
static LAST_TRY: Mutex<Option<Instant>> = Mutex::new(None);

/// Send a copy of the whole site if the last one is a day old. Called every
/// hour; does nothing while copying is off.
pub async fn run_daily_copy(pool: &SqlitePool, uploads: &std::path::Path) {
    let site = settings::load(pool).await;
    let Some(storage) = Storage::from_settings(&site) else {
        return;
    };
    let copied_recently = crate::content::text::parse_datetime(&site.storage_last_copied)
        .is_some_and(|last| chrono::Utc::now().naive_utc() - last < chrono::Duration::days(1));
    let tried_recently = LAST_TRY
        .lock()
        .ok()
        .and_then(|last| *last)
        .is_some_and(|last| last.elapsed() < DAY);
    if copied_recently || tried_recently {
        return;
    }
    if let Ok(mut last) = LAST_TRY.lock() {
        *last = Some(Instant::now());
    }
    copy_backup(pool, &storage, uploads).await;
}

/// Make the backup .zip, send it, and remember how it went (shown in
/// Settings → Backup and on the Dashboard).
async fn copy_backup(pool: &SqlitePool, storage: &Storage, uploads: &std::path::Path) {
    let archive = crate::services::backup::timestamped_path("storage-").with_extension("zip");
    let result = match crate::services::backup::write_backup(pool, uploads, &archive).await {
        Ok(()) => {
            let name = format!(
                "bloogla-{}.zip",
                chrono::Utc::now().format("%A").to_string().to_lowercase()
            );
            let (storage, path) = (storage_copy(storage), archive.clone());
            tokio::task::spawn_blocking(move || upload_file(&storage, &name, &path))
                .await
                .unwrap_or_else(|e| Err(Problem::Other(e.to_string())))
        }
        Err(e) => Err(Problem::Other(e)),
    };
    let _ = std::fs::remove_file(&archive);

    let values = match &result {
        Ok(()) => {
            console::activity_t(Kind::Done, "Backup copied to your storage.");
            vec![
                (
                    "storage_last_copied",
                    chrono::Utc::now().format("%Y-%m-%d %H:%M:%S").to_string(),
                ),
                ("storage_last_problem", String::new()),
            ]
        }
        Err(problem) => {
            tracing::error!("Copying the backup to storage failed: {}", problem.detail());
            console::activity_t(
                Kind::Problem,
                "The backup couldn't be copied to your storage. See Settings → Backup.",
            );
            vec![("storage_last_problem", problem.message().to_string())]
        }
    };
    if let Err(e) = settings::save(pool, &values).await {
        tracing::error!("Could not save how the storage copy went: {e}");
    }
}

/// [`Storage`] has no `Clone` on purpose (it holds a secret); this makes the
/// one copy that moves to the upload thread.
fn storage_copy(storage: &Storage) -> Storage {
    Storage {
        endpoint: storage.endpoint.clone(),
        region: storage.region.clone(),
        bucket: storage.bucket.clone(),
        access_key: storage.access_key.clone(),
        secret_key: storage.secret_key.clone(),
    }
}

/// Send a small file, to check the address, bucket and keys when they're saved.
pub async fn check(storage: Storage) -> Result<(), Problem> {
    tokio::task::spawn_blocking(move || {
        let body = b"Bloogla checks that it can save backups here. This file can be deleted.";
        put(&storage, "bloogla-check.txt", Body::Bytes(body))
    })
    .await
    .unwrap_or_else(|e| Err(Problem::Other(e.to_string())))
}

/// Why sending failed. `message` is shown to the owner (and translated),
/// `detail` goes to the log.
#[derive(Debug)]
pub enum Problem {
    Address,
    Unreachable(String),
    Keys(String),
    Bucket(String),
    Other(String),
}

impl Problem {
    pub fn message(&self) -> &'static str {
        match self {
            Problem::Address => "The storage address must start with https://.",
            Problem::Unreachable(_) => "Bloogla couldn't reach your storage. Check the storage address.",
            Problem::Keys(_) => "Your storage didn't accept the keys. Check the access key, the secret key and the region.",
            Problem::Bucket(_) => "Your storage has no bucket with that name. Check the bucket name.",
            Problem::Other(_) => "Your storage didn't accept the backup. The details are in Bloogla's log.",
        }
    }

    fn detail(&self) -> &str {
        match self {
            Problem::Address => "not an https:// address",
            Problem::Unreachable(d) | Problem::Keys(d) | Problem::Bucket(d) | Problem::Other(d) => {
                d
            }
        }
    }
}

/// What a request sends.
enum Body<'a> {
    Bytes(&'a [u8]),
    File(std::fs::File),
}

fn upload_file(storage: &Storage, name: &str, path: &std::path::Path) -> Result<(), Problem> {
    let file = std::fs::File::open(path).map_err(|e| Problem::Other(e.to_string()))?;
    put(storage, name, Body::File(file))
}

/// `PUT` one file into the bucket, signed with the storage's keys.
fn put(storage: &Storage, name: &str, body: Body) -> Result<(), Problem> {
    let Some(host) = storage.endpoint.strip_prefix("https://") else {
        return Err(Problem::Address);
    };
    if host.is_empty() || host.contains('/') {
        return Err(Problem::Address);
    }
    // "Path style" (`host/bucket/name`) works with every provider.
    let path = format!("/{}/{name}", uri_encode(&storage.bucket));
    let now = chrono::Utc::now();
    let amz_date = now.format("%Y%m%dT%H%M%SZ").to_string();
    // The file can be large, so its hash isn't part of the signature; HTTPS
    // already protects it on the way.
    let payload = "UNSIGNED-PAYLOAD";
    let headers = [
        ("host", host),
        ("x-amz-content-sha256", payload),
        ("x-amz-date", amz_date.as_str()),
    ];
    let authorization = authorization(storage, "PUT", &path, &headers, payload, &amz_date);

    let agent: ureq::Agent = ureq::Agent::config_builder()
        .timeout_global(Some(UPLOAD_TIME_LIMIT))
        .timeout_connect(Some(Duration::from_secs(20)))
        // Keys are signed for this address only; never follow a redirect elsewhere.
        .max_redirects(0)
        .http_status_as_error(false)
        .user_agent(format!(
            "Bloogla/{}",
            crate::services::updates::CURRENT_VERSION
        ))
        .build()
        .into();
    let request = agent
        .put(format!("https://{host}{path}"))
        .header("x-amz-content-sha256", payload)
        .header("x-amz-date", &amz_date)
        .header("authorization", &authorization);
    let response = match body {
        Body::Bytes(bytes) => request.header("content-type", "text/plain").send(bytes),
        Body::File(file) => request.header("content-type", "application/zip").send(file),
    };
    let mut response = response.map_err(|e| Problem::Unreachable(e.to_string()))?;

    let status = response.status().as_u16();
    if (200..300).contains(&status) {
        return Ok(());
    }
    // S3 explains refusals in a short XML document.
    let answer = response
        .body_mut()
        .with_config()
        .limit(64 * 1024)
        .read_to_string()
        .unwrap_or_default();
    let detail = format!("{status}: {answer}");
    Err(match status {
        403 => Problem::Keys(detail),
        // A wrong region is "AuthorizationHeaderMalformed" with a 400.
        400 if answer.contains("AuthorizationHeader") || answer.contains("Region") => {
            Problem::Keys(detail)
        }
        404 if answer.contains("NoSuchBucket") => Problem::Bucket(detail),
        _ => Problem::Other(detail),
    })
}

/// The `Authorization` header for a request (AWS Signature Version 4).
/// `headers` must be sorted by name and in lower case; all of them are signed.
fn authorization(
    storage: &Storage,
    method: &str,
    path: &str,
    headers: &[(&str, &str)],
    payload_hash: &str,
    amz_date: &str,
) -> String {
    let date = amz_date.get(..8).unwrap_or_default();
    let scope = format!("{date}/{}/s3/aws4_request", storage.region);

    let canonical_headers: String = headers
        .iter()
        .map(|(name, value)| format!("{name}:{}\n", value.trim()))
        .collect();
    let signed_headers = headers
        .iter()
        .map(|(name, _)| *name)
        .collect::<Vec<_>>()
        .join(";");
    // Method, path, query (none), headers, signed header names, body hash.
    let canonical_request =
        format!("{method}\n{path}\n\n{canonical_headers}\n{signed_headers}\n{payload_hash}");
    let string_to_sign = format!(
        "AWS4-HMAC-SHA256\n{amz_date}\n{scope}\n{}",
        to_hex(&Sha256::digest(canonical_request.as_bytes()))
    );

    // The signing key is the secret, narrowed down step by step to today,
    // this region and S3.
    let mut key = hmac(format!("AWS4{}", storage.secret_key).as_bytes(), date);
    for part in [storage.region.as_str(), "s3", "aws4_request"] {
        key = hmac(&key, part);
    }
    let signature = to_hex(&hmac(&key, &string_to_sign));

    format!(
        "AWS4-HMAC-SHA256 Credential={}/{scope}, SignedHeaders={signed_headers}, Signature={signature}",
        storage.access_key
    )
}

fn hmac(key: &[u8], text: &str) -> Vec<u8> {
    let mut mac = Hmac::<Sha256>::new_from_slice(key).expect("HMAC accepts any key length");
    mac.update(text.as_bytes());
    mac.finalize().into_bytes().to_vec()
}

/// Percent-encode a bucket name the way S3 signs it (letters, digits and
/// `-_.~` stay as they are).
fn uri_encode(text: &str) -> String {
    text.bytes()
        .map(|b| {
            if b.is_ascii_alphanumeric() || b"-_.~".contains(&b) {
                (b as char).to_string()
            } else {
                format!("%{b:02X}")
            }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The worked example in Amazon's S3 documentation ("GET Object"), so the
    /// signature is known to be right.
    #[test]
    fn signs_like_amazons_example() {
        let storage = Storage {
            endpoint: "https://examplebucket.s3.amazonaws.com".into(),
            region: "us-east-1".into(),
            bucket: String::new(),
            access_key: "AKIAIOSFODNN7EXAMPLE".into(),
            secret_key: "wJalrXUtnFEMI/K7MDENG/bPxRfiCYEXAMPLEKEY".into(),
        };
        let empty = "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855";
        let headers = [
            ("host", "examplebucket.s3.amazonaws.com"),
            ("range", "bytes=0-9"),
            ("x-amz-content-sha256", empty),
            ("x-amz-date", "20130524T000000Z"),
        ];
        let header = authorization(
            &storage,
            "GET",
            "/test.txt",
            &headers,
            empty,
            "20130524T000000Z",
        );
        assert_eq!(
            header,
            "AWS4-HMAC-SHA256 Credential=AKIAIOSFODNN7EXAMPLE/20130524/us-east-1/s3/aws4_request, \
             SignedHeaders=host;range;x-amz-content-sha256;x-amz-date, \
             Signature=f0e8bdb87c964420e857bd35b5d6ed310bd44f0170aba48dd91039c6036bdb41"
        );
    }

    #[test]
    fn only_https_addresses_are_used() {
        let storage = Storage {
            endpoint: "http://s3.example.com".into(),
            region: "auto".into(),
            bucket: "b".into(),
            access_key: "a".into(),
            secret_key: "s".into(),
        };
        assert!(matches!(
            put(&storage, "x.txt", Body::Bytes(b"x")),
            Err(Problem::Address)
        ));
    }
}
