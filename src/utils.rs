use chrono::NaiveDateTime;
use comrak::{markdown_to_html, ComrakOptions};
use sqlx::SqlitePool;

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

/// Fetch a setting from the database with a fallback default.
/// Logs database errors while keeping resilience through defaults.
pub async fn get_setting(pool: &SqlitePool, key: &str, default: &str) -> String {
    match sqlx::query_scalar::<_, String>("SELECT value FROM settings WHERE key = ?")
        .bind(key)
        .fetch_optional(pool)
        .await
    {
        Ok(Some(value)) => value,
        Ok(None) => default.to_string(),
        Err(e) => {
            eprintln!("Database error fetching setting key '{key}': {e}");
            default.to_string()
        }
    }
}

/// Convert a string into a URL-friendly slug.
pub fn slugify(title: &str) -> String {
    let mut slug = String::new();
    let mut last_was_dash = false;

    for c in title.to_lowercase().chars() {
        if c.is_ascii_alphanumeric() {
            slug.push(c);
            last_was_dash = false;
        } else if !last_was_dash {
            slug.push('-');
            last_was_dash = true;
        }
    }

    let trimmed = slug.trim_matches('-').to_string();
    if trimmed.is_empty() {
        "article".to_string()
    } else {
        trimmed
    }
}

/// Generate a unique slug for an article, appending an incremental counter if needed.
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
            sqlx::query_scalar("SELECT id FROM articles WHERE slug = ? AND (? IS NULL OR id != ?)")
                .bind(&candidate)
                .bind(exclude_id)
                .bind(exclude_id)
                .fetch_optional(pool)
                .await?;

        match exists {
            None => return Ok(candidate),
            Some(_) => {
                candidate = format!("{}-{}", base, counter);
                counter += 1;
            }
        }
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
