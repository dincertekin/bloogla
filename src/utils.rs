use chrono::NaiveDateTime;
use comrak::{markdown_to_html, ComrakOptions};
use sqlx::SqlitePool;
use std::collections::HashMap;

/// Convert raw Markdown input into sanitized HTML.
///
/// Uses `comrak` to render the Markdown and `ammonia` to sanitize
/// the output against XSS vulnerabilities.
pub fn render_safe_markdown(markdown_input: &str) -> String {
    let mut options = ComrakOptions::default();

    options.extension.strikethrough = true;
    options.extension.tagfilter = false;
    options.extension.table = true;
    options.extension.autolink = true;
    options.extension.tasklist = true;
    options.extension.header_ids = Some("".to_string());

    options.render.unsafe_ = true;

    let raw_html = markdown_to_html(markdown_input, &options);

    ammonia::Builder::default()
        .add_tags(&["details", "summary", "kbd", "mark", "sub", "sup", "input"])
        .add_generic_attributes(&["class", "id"])
        .add_tag_attributes("input", &["type", "checked", "disabled"])
        .add_tag_attribute_values("input", "type", &["checkbox"])
        .clean(&raw_html)
        .to_string()
}

/// Extract the readable text of a Markdown document, dropping markup and raw HTML.
pub fn markdown_to_plain_text(markdown_input: &str) -> String {
    use comrak::nodes::NodeValue;

    let arena = comrak::Arena::new();
    let mut options = ComrakOptions::default();
    options.extension.strikethrough = true;
    options.extension.table = true;
    options.extension.autolink = true;
    let root = comrak::parse_document(&arena, markdown_input, &options);

    let mut text = String::new();
    for node in root.descendants() {
        match &node.data.borrow().value {
            NodeValue::Text(t) => text.push_str(t),
            NodeValue::Code(c) => text.push_str(&c.literal),
            NodeValue::SoftBreak | NodeValue::LineBreak => text.push(' '),
            NodeValue::Paragraph | NodeValue::Heading(_) | NodeValue::Item(_)
                if !text.is_empty() && !text.ends_with(' ') =>
            {
                text.push(' ');
            }
            _ => {}
        }
    }
    text.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// Plain-text excerpt of a Markdown document, cut at a word boundary.
pub fn excerpt(markdown_input: &str, max_chars: usize) -> String {
    let text = markdown_to_plain_text(markdown_input);
    if text.chars().count() <= max_chars {
        return text;
    }
    let cut: String = text.chars().take(max_chars).collect();
    let cut = match cut.rfind(' ') {
        Some(i) if i > max_chars / 2 => &cut[..i],
        _ => &cut,
    };
    format!(
        "{}…",
        cut.trim_end_matches(|c: char| c.is_ascii_punctuation())
    )
}

/// Turn free text into a safe SQLite FTS5 query: every word must appear, and
/// each word also matches longer words starting with it ("istan" → "istanbul").
///
/// Returns `None` when the text has no searchable words.
pub fn fts_query(input: &str) -> Option<String> {
    let terms: Vec<String> = input
        .split(|c: char| !c.is_alphanumeric())
        .filter(|w| !w.is_empty())
        .take(12)
        .map(|w| format!("\"{}\"*", w.to_lowercase()))
        .collect();
    (!terms.is_empty()).then(|| terms.join(" "))
}

/// Percent-encode a string for use in a URL query value.
pub fn url_encode(input: &str) -> String {
    input
        .bytes()
        .map(|b| match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                (b as char).to_string()
            }
            _ => format!("%{b:02X}"),
        })
        .collect()
}

/// Decode `%XX` sequences (WordPress stores non-ASCII slugs percent-encoded).
pub fn percent_decode(input: &str) -> String {
    fn hex(b: u8) -> Option<u8> {
        (b as char).to_digit(16).map(|d| d as u8)
    }

    let bytes = input.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%' && i + 2 < bytes.len() {
            if let (Some(hi), Some(lo)) = (hex(bytes[i + 1]), hex(bytes[i + 2])) {
                out.push(hi << 4 | lo);
                i += 3;
                continue;
            }
        }
        out.push(bytes[i]);
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

/// Parse the navigation menu setting: one `Label | URL` per line.
pub fn parse_menu(raw: &str) -> Vec<crate::models::MenuItem> {
    raw.lines()
        .filter_map(|line| {
            let (label, url) = line.split_once('|')?;
            let (label, url) = (label.trim(), url.trim());
            let safe_url =
                url.starts_with('/') || url.starts_with("https://") || url.starts_with("http://");
            (!label.is_empty() && safe_url).then(|| crate::models::MenuItem {
                label: label.to_string(),
                url: url.to_string(),
            })
        })
        .collect()
}

/// Calculate the estimated reading time of a text string in minutes.
///
/// Assumes an average reading speed of 200 words per minute.
/// Always returns at least 1 minute.
pub fn calculate_reading_time(text: &str) -> u32 {
    let word_count = text.split_whitespace().count();
    let minutes = (word_count as f32 / 200.0).ceil() as u32;
    if minutes == 0 {
        1
    } else {
        minutes
    }
}

/// Format a date string into `Mon DD, YYYY` format.
///
/// Falls back to returning the date portion if parsing fails.
pub fn format_display_date(raw_date: &str) -> String {
    if let Ok(dt) = NaiveDateTime::parse_from_str(raw_date, "%Y-%m-%d %H:%M:%S") {
        dt.format("%b %d, %Y").to_string()
    } else if let Ok(dt) = NaiveDateTime::parse_from_str(raw_date, "%Y-%m-%dT%H:%M:%S") {
        dt.format("%b %d, %Y").to_string()
    } else {
        raw_date
            .split(&[' ', 'T'][..])
            .next()
            .unwrap_or(raw_date)
            .to_string()
    }
}

/// In-memory copy of the `settings` table. Pages read several settings per
/// request, so they are loaded once and reloaded after any change.
static SETTINGS: std::sync::RwLock<Option<HashMap<String, String>>> = std::sync::RwLock::new(None);

/// Format a stored UTC timestamp as `Oct 02, 14:30 UTC`.
pub fn format_display_datetime(raw_date: &str) -> String {
    NaiveDateTime::parse_from_str(&raw_date.replace('T', " "), "%Y-%m-%d %H:%M:%S")
        .map(|dt| dt.format("%b %d, %H:%M UTC").to_string())
        .unwrap_or_else(|_| raw_date.to_string())
}

/// Fetch a setting with a fallback default.
pub async fn get_setting(pool: &SqlitePool, key: &str, default: &str) -> String {
    if let Ok(cache) = SETTINGS.read() {
        if let Some(map) = cache.as_ref() {
            return map.get(key).cloned().unwrap_or_else(|| default.to_string());
        }
    }

    match sqlx::query_as::<_, (String, String)>("SELECT key, value FROM settings")
        .fetch_all(pool)
        .await
    {
        Ok(rows) => {
            let map: HashMap<String, String> = rows.into_iter().collect();
            let value = map.get(key).cloned().unwrap_or_else(|| default.to_string());
            if let Ok(mut cache) = SETTINGS.write() {
                *cache = Some(map);
            }
            value
        }
        Err(e) => {
            eprintln!("Database error fetching setting key '{key}': {e}");
            default.to_string()
        }
    }
}

/// Save a setting and refresh the cache.
pub async fn set_setting(pool: &SqlitePool, key: &str, value: &str) -> Result<(), sqlx::Error> {
    let result = sqlx::query(
        "INSERT INTO settings (key, value) VALUES (?, ?)
         ON CONFLICT(key) DO UPDATE SET value = excluded.value",
    )
    .bind(key)
    .bind(value)
    .execute(pool)
    .await;
    invalidate_settings();
    result.map(|_| ())
}

/// Drop cached settings so the next read sees the database.
pub fn invalidate_settings() {
    if let Ok(mut cache) = SETTINGS.write() {
        *cache = None;
    }
}

/// Unwrap a database result, logging the error and falling back to the default value.
pub fn or_log<T: Default>(result: Result<T, sqlx::Error>, context: &str) -> T {
    result.unwrap_or_else(|e| {
        eprintln!("Database error ({context}): {e}");
        T::default()
    })
}

/// Restrict an post status to a known value, defaulting to "published".
pub fn normalize_status(status: Option<String>) -> String {
    match status.as_deref().map(str::trim) {
        Some(s @ ("draft" | "scheduled")) => s.to_string(),
        _ => "published".to_string(),
    }
}

/// Normalize a submitted publish date (e.g. `datetime-local` input) to
/// `YYYY-MM-DD HH:MM:SS`, falling back to the current UTC time.
pub fn normalize_published_at(input: Option<String>) -> String {
    parse_datetime(input.as_deref().unwrap_or_default())
        .unwrap_or_else(|| chrono::Utc::now().naive_utc())
        .format("%Y-%m-%d %H:%M:%S")
        .to_string()
}

/// Parse a stored or submitted date, with or without seconds and `T` separator.
fn parse_datetime(input: &str) -> Option<NaiveDateTime> {
    let input = input.trim();
    [
        "%Y-%m-%d %H:%M:%S",
        "%Y-%m-%dT%H:%M:%S",
        "%Y-%m-%d %H:%M",
        "%Y-%m-%dT%H:%M",
    ]
    .iter()
    .find_map(|fmt| NaiveDateTime::parse_from_str(input, fmt).ok())
}

/// Convert a stored `YYYY-MM-DD HH:MM:SS` date into a `datetime-local` input value.
pub fn to_datetime_local(raw_date: &str) -> String {
    raw_date.replace(' ', "T").chars().take(16).collect()
}

/// Convert a stored `YYYY-MM-DD HH:MM:SS` (UTC) date into RFC 2822 for RSS.
pub fn to_rfc2822(raw_date: &str) -> String {
    parse_datetime(raw_date)
        .map(|dt| dt.and_utc().to_rfc2822())
        .unwrap_or_else(|| raw_date.to_string())
}

/// Convert a string into a URL-friendly slug.
///
/// Accented Latin letters are transliterated (`ç` → `c`, `ş` → `s`); letters
/// from other scripts are kept as-is.
pub fn slugify(title: &str) -> String {
    slugify_or(title, "post")
}

/// [`slugify`], using `fallback` when nothing usable is left.
pub fn slugify_or(title: &str, fallback: &str) -> String {
    let mut slug = String::new();
    let mut last_was_dash = false;

    for c in title.to_lowercase().chars() {
        // Combining marks, e.g. the dot left by lowercasing `İ`.
        if ('\u{300}'..='\u{36f}').contains(&c) {
            continue;
        }
        let ascii = transliterate(c);
        if !ascii.is_empty() {
            slug.push_str(ascii);
            last_was_dash = false;
        } else if c.is_alphanumeric() {
            slug.push(c);
            last_was_dash = false;
        } else if !last_was_dash {
            slug.push('-');
            last_was_dash = true;
        }
    }

    let trimmed = slug.trim_matches('-').to_string();
    if trimmed.is_empty() {
        fallback.to_string()
    } else {
        trimmed
    }
}

/// ASCII spelling of a lowercase character, or `""` if it has none.
fn transliterate(c: char) -> &'static str {
    match c {
        'a'..='z' | '0'..='9' => {
            const ASCII: &str = "abcdefghijklmnopqrstuvwxyz0123456789";
            let i = ASCII.find(c).unwrap_or(0);
            &ASCII[i..i + 1]
        }
        'à' | 'á' | 'â' | 'ã' | 'ä' | 'å' | 'ā' | 'ă' | 'ą' => "a",
        'ç' | 'ć' | 'č' => "c",
        'ď' | 'đ' => "d",
        'è' | 'é' | 'ê' | 'ë' | 'ē' | 'ę' | 'ě' => "e",
        'ğ' => "g",
        'ì' | 'í' | 'î' | 'ï' | 'ı' | 'ī' => "i",
        'ł' => "l",
        'ñ' | 'ń' | 'ň' => "n",
        'ò' | 'ó' | 'ô' | 'õ' | 'ö' | 'ø' | 'ō' | 'ő' => "o",
        'ř' => "r",
        'ş' | 'ś' | 'š' | 'ș' => "s",
        'ț' | 'ť' => "t",
        'ù' | 'ú' | 'û' | 'ü' | 'ū' | 'ů' | 'ű' => "u",
        'ý' | 'ÿ' => "y",
        'ź' | 'ż' | 'ž' => "z",
        'ß' => "ss",
        'æ' => "ae",
        'œ' => "oe",
        _ => "",
    }
}

/// Slugs that would be shadowed by built-in routes when used for a page.
const RESERVED_SLUGS: &[&str] = &[
    "admin",
    "post",
    "article",
    "tag",
    "search",
    "health",
    "static",
    "uploads",
    "theme-assets",
    "setup",
];

/// Generate a unique slug for an post, appending an incremental counter if needed.
pub async fn generate_unique_slug(
    pool: &SqlitePool,
    title: &str,
    exclude_id: Option<i64>,
) -> Result<String, sqlx::Error> {
    let base = slugify(title);
    let mut candidate = base.clone();
    let mut counter = 2;

    loop {
        let exists: Option<i64> =
            sqlx::query_scalar("SELECT id FROM posts WHERE slug = ? AND (? IS NULL OR id != ?)")
                .bind(&candidate)
                .bind(exclude_id)
                .bind(exclude_id)
                .fetch_optional(pool)
                .await?;

        if exists.is_none() && !RESERVED_SLUGS.contains(&candidate.as_str()) {
            return Ok(candidate);
        }
        candidate = format!("{}-{}", base, counter);
        counter += 1;
    }
}

/// Escape XML characters for RSS feeds and Sitemaps.
pub fn xml_escape(input: &str) -> String {
    input
        .replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&apos;")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn slugify_transliterates_and_keeps_other_scripts() {
        assert_eq!(slugify("Dinçer'in Güzel Şehri"), "dincer-in-guzel-sehri");
        assert_eq!(slugify("İstanbul ılık"), "istanbul-ilik");
        assert_eq!(slugify("  Hello, World!  "), "hello-world");
        assert_eq!(slugify("Привет мир"), "привет-мир");
        assert_eq!(slugify("!!!"), "post");
    }

    #[test]
    fn excerpt_strips_markup_and_cuts_on_words() {
        assert_eq!(
            excerpt("# Title\n\nSome **bold** `code`.", 100),
            "Title Some bold code."
        );
        assert_eq!(excerpt("one two three four", 9), "one two…");
    }

    #[test]
    fn decodes_percent_encoded_slugs() {
        assert_eq!(percent_decode("merhaba-d%c3%bcnya"), "merhaba-dünya");
        assert_eq!(percent_decode("100%"), "100%");
        assert_eq!(percent_decode("a%zzb"), "a%zzb");
        assert_eq!(percent_decode("%ç1"), "%ç1");
    }

    #[test]
    fn fts_query_quotes_words_and_drops_syntax() {
        assert_eq!(fts_query("Rust  web"), Some("\"rust\"* \"web\"*".into()));
        assert_eq!(fts_query("a\" OR b*"), Some("\"a\"* \"or\"* \"b\"*".into()));
        assert_eq!(fts_query("!!! ---"), None);
    }

    #[test]
    fn menu_rejects_unsafe_urls() {
        let menu = parse_menu("About | /about\nBad | javascript:alert(1)\nGH | https://github.com");
        let urls: Vec<_> = menu.iter().map(|m| m.url.as_str()).collect();
        assert_eq!(urls, ["/about", "https://github.com"]);
    }
}
