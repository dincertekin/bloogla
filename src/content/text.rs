//! Small text helpers: slugs, URL encoding, escaping and dates.

use crate::i18n::Lang;
use chrono::NaiveDateTime;
use chrono_tz::Tz;

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

    let trimmed = slug.trim_matches('-');
    if trimmed.is_empty() {
        fallback.to_string()
    } else {
        trimmed.to_string()
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

/// Percent-encode a string for use in a URL path or query value.
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

/// Escape text for HTML and XML (attributes included).
pub fn escape_html(input: &str) -> String {
    let mut out = String::with_capacity(input.len());
    for c in input.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            '\'' => out.push_str("&apos;"),
            _ => out.push(c),
        }
    }
    out
}

/// Estimated reading time in minutes (200 words per minute, at least 1).
pub fn reading_time(text: &str) -> u32 {
    let words = text.split_whitespace().count() as u32;
    words.div_ceil(200).max(1)
}

/// Parse a stored or submitted date, with or without seconds and `T` separator.
pub fn parse_datetime(input: &str) -> Option<NaiveDateTime> {
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

/// Normalize a submitted date (e.g. a `datetime-local` input) to the stored
/// `YYYY-MM-DD HH:MM:SS` form, falling back to the current UTC time.
///
/// `input` is a date and time typed in the site's time zone `tz` (from the
/// editor); the result is in UTC, as stored.
pub fn normalize_datetime(input: Option<&str>, tz: Tz) -> String {
    let utc = match input.and_then(parse_datetime) {
        Some(local) => local_to_utc(local, tz),
        None => chrono::Utc::now().naive_utc(),
    };
    utc.format("%Y-%m-%d %H:%M:%S").to_string()
}

/// A local time in `tz` as UTC. When clocks change, a time that happens twice
/// takes the first, and one that's skipped (it doesn't exist) the hour after.
fn local_to_utc(local: NaiveDateTime, tz: Tz) -> NaiveDateTime {
    use chrono::TimeZone;
    tz.from_local_datetime(&local)
        .earliest()
        .or_else(|| {
            tz.from_local_datetime(&(local + chrono::Duration::hours(1)))
                .earliest()
        })
        .map_or(local, |dt| dt.naive_utc())
}

/// A stored UTC time in the site's time zone.
fn to_local(utc: NaiveDateTime, tz: Tz) -> NaiveDateTime {
    utc.and_utc().with_timezone(&tz).naive_local()
}

/// Format a stored date for display, in the site's time zone:
/// `Oct 2, 2026` / `2 Eki 2026`.
pub fn display_date(lang: Lang, tz: Tz, raw_date: &str) -> String {
    match parse_datetime(raw_date) {
        Some(dt) => lang.date(to_local(dt, tz).date()),
        None => raw_date
            .split([' ', 'T'])
            .next()
            .unwrap_or(raw_date)
            .to_string(),
    }
}

/// Format a stored timestamp with its time, in the site's time zone:
/// `Oct 2, 14:30` / `2 Eki, 14:30`.
pub fn display_datetime(lang: Lang, tz: Tz, raw_date: &str) -> String {
    parse_datetime(raw_date)
        .map(|dt| {
            let local = to_local(dt, tz);
            format!(
                "{}, {}",
                lang.day_month(local.date()),
                local.format("%H:%M")
            )
        })
        .unwrap_or_else(|| raw_date.to_string())
}

/// Stored date → `datetime-local` input value in the site's time zone
/// (`2026-10-02T14:30`).
pub fn to_datetime_local(tz: Tz, raw_date: &str) -> String {
    match parse_datetime(raw_date) {
        Some(dt) => to_local(dt, tz).format("%Y-%m-%dT%H:%M").to_string(),
        None => String::new(),
    }
}

/// Stored UTC date → RFC 2822, for RSS.
pub fn to_rfc2822(raw_date: &str) -> String {
    parse_datetime(raw_date)
        .map(|dt| dt.and_utc().to_rfc2822())
        .unwrap_or_else(|| raw_date.to_string())
}

/// Stored UTC date → ISO 8601 (`2026-10-02T14:30:00Z`), for SEO and feeds.
pub fn to_iso8601(raw_date: &str) -> String {
    format!("{}Z", raw_date.replacen(' ', "T", 1))
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
    fn escapes_html() {
        assert_eq!(
            escape_html(r#"<a href="x">Tom & Jerry's</a>"#),
            "&lt;a href=&quot;x&quot;&gt;Tom &amp; Jerry&apos;s&lt;/a&gt;"
        );
    }

    #[test]
    fn dates_are_shown_and_typed_in_the_site_time_zone() {
        let istanbul: Tz = "Europe/Istanbul".parse().unwrap();
        // 09:00 in Istanbul (UTC+3) is stored as 06:00 UTC, and shown as 09:00.
        let stored = normalize_datetime(Some("2026-10-02T09:00"), istanbul);
        assert_eq!(stored, "2026-10-02 06:00:00");
        assert_eq!(to_datetime_local(istanbul, &stored), "2026-10-02T09:00");
        assert_eq!(
            display_datetime(Lang::default(), istanbul, &stored),
            "Oct 2, 09:00"
        );
        // Late at night in UTC is already the next day in Istanbul.
        assert_eq!(
            display_date(Lang::default(), istanbul, "2026-10-02 22:30:00"),
            "Oct 3, 2026"
        );
        // Daylight saving: 02:30 doesn't exist in Berlin on March 29, 2026.
        let berlin: Tz = "Europe/Berlin".parse().unwrap();
        assert_eq!(
            normalize_datetime(Some("2026-03-29T02:30"), berlin),
            "2026-03-29 01:30:00"
        );
    }

    #[test]
    fn reading_time_is_at_least_a_minute() {
        assert_eq!(reading_time(""), 1);
        assert_eq!(reading_time(&"word ".repeat(201)), 2);
    }
}
