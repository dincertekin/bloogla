//! Markdown → safe HTML, and Markdown → plain text.

use comrak::{markdown_to_html, ComrakOptions};
use std::sync::OnceLock;

/// Convert Markdown into HTML that is safe to show to visitors.
///
/// `comrak` renders the Markdown (raw HTML allowed), then `ammonia` removes
/// anything that could run scripts or break the page (XSS protection).
pub fn to_safe_html(markdown: &str) -> String {
    let mut options = ComrakOptions::default();
    options.extension.strikethrough = true;
    options.extension.table = true;
    options.extension.autolink = true;
    options.extension.tasklist = true;
    options.extension.header_ids = Some(String::new());
    // ammonia sanitizes the result, so raw HTML can be let through here.
    options.render.unsafe_ = true;

    let html = sanitizer()
        .clean(&markdown_to_html(markdown, &options))
        .to_string();
    // Images in posts load when they're about to scroll into view.
    let html = html.replace("<img ", r#"<img loading="lazy" decoding="async" "#);
    move_heading_ids(&html)
}

/// comrak marks each heading with an empty link (`<h2><a href="#costs"
/// class="anchor" id="costs"></a>Costs</h2>`). Put the id on the heading
/// instead, so `#costs` links still work but there's no empty link, which
/// screen readers can't name.
fn move_heading_ids(html: &str) -> String {
    let mut out = String::with_capacity(html.len());
    let mut rest = html;
    while let Some(start) = rest.find("<a href=\"#") {
        let before = &rest[..start];
        let Some(tag_end) = rest[start..].find('>') else {
            break;
        };
        let tag = &rest[start..start + tag_end + 1];
        let after = &rest[start + tag.len()..];
        let id = tag.split("id=\"").nth(1).and_then(|s| s.split('"').next());
        let in_heading = (1..=6).any(|n| before.ends_with(&format!("<h{n}>")));

        match id {
            Some(id)
                if in_heading && tag.contains("class=\"anchor\"") && after.starts_with("</a>") =>
            {
                // `before` ends with `<hN>`: reopen it with the id.
                out.push_str(&before[..before.len() - 1]);
                out.push_str(&format!(" id=\"{id}\">"));
                rest = &after["</a>".len()..];
            }
            _ => {
                out.push_str(&rest[..start + tag.len()]);
                rest = after;
            }
        }
    }
    out.push_str(rest);
    out
}

/// The HTML sanitizer, built once.
fn sanitizer() -> &'static ammonia::Builder<'static> {
    static SANITIZER: OnceLock<ammonia::Builder<'static>> = OnceLock::new();
    SANITIZER.get_or_init(|| {
        let mut builder = ammonia::Builder::default();
        builder
            .add_tags(["details", "summary", "kbd", "mark", "sub", "sup", "input"])
            .add_generic_attributes(["class", "id"])
            .add_tag_attributes("input", ["type", "checked", "disabled"])
            .add_tag_attribute_values("input", "type", ["checkbox"]);
        builder
    })
}

/// The readable text of a Markdown document, without markup or raw HTML.
pub fn to_plain_text(markdown: &str) -> String {
    use comrak::nodes::NodeValue;

    let arena = comrak::Arena::new();
    let mut options = ComrakOptions::default();
    options.extension.strikethrough = true;
    options.extension.table = true;
    options.extension.autolink = true;
    let root = comrak::parse_document(&arena, markdown, &options);

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
pub fn excerpt(markdown: &str, max_chars: usize) -> String {
    let text = super::shortcodes::strip(&to_plain_text(markdown));
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn excerpt_strips_markup_and_cuts_on_words() {
        assert_eq!(
            excerpt("# Title\n\nSome **bold** `code`.", 100),
            "Title Some bold code."
        );
        assert_eq!(excerpt("one two three four", 9), "one two…");
    }

    #[test]
    fn headings_carry_their_own_ids() {
        let html = to_safe_html("## Getting around\n\nSee [costs](#costs).\n\n## Costs");
        assert!(
            html.contains(r#"<h2 id="getting-around">Getting around</h2>"#),
            "{html}"
        );
        assert!(html.contains(r#"<h2 id="costs">Costs</h2>"#), "{html}");
        assert!(html.contains(r##"<a href="#costs""##), "{html}");
        assert!(!html.contains("class=\"anchor\""), "{html}");
    }

    #[test]
    fn images_load_lazily() {
        assert!(to_safe_html("![a](/uploads/x.jpg)")
            .contains(r#"<img loading="lazy" decoding="async" src="/uploads/x.jpg""#));
    }

    #[test]
    fn scripts_are_removed() {
        let html = to_safe_html("Hi <script>alert(1)</script><img src=x onerror=alert(1)>");
        assert!(!html.contains("<script") && !html.contains("onerror"));
    }
}
