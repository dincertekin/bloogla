use chrono::NaiveDateTime;
use comrak::{markdown_to_html, ComrakOptions};

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
    let mut builder = ammonia::Builder::default();
    builder
        .add_tags(&["details", "summary", "kbd", "mark", "sub", "sup", "input"])
        .add_generic_attributes(&["class", "id", "type", "checked", "disabled"]);

    builder.clean(&raw_html).to_string()
}

pub fn calculate_reading_time(text: &str) -> u32 {
    let word_count = text.split_whitespace().count();
    let minutes = (word_count as f32 / 200.0).ceil() as u32;
    if minutes == 0 {
        1
    } else {
        minutes
    }
}

pub fn format_display_date(raw_date: &str) -> String {
    if let Ok(dt) = NaiveDateTime::parse_from_str(raw_date, "%Y-%m-%d %H:%M:%S") {
        dt.format("%b %d, %Y").to_string()
    } else {
        raw_date.split(' ').next().unwrap_or(raw_date).to_string()
    }
}
