//! Markdown → safe HTML, and Markdown → plain text.

use comrak::{markdown_to_html, Options};
use std::sync::OnceLock;

/// Convert Markdown into HTML that is safe to show to visitors.
///
/// `comrak` renders the Markdown (raw HTML allowed), then `ammonia` removes
/// anything that could run scripts or break the page (XSS protection).
pub fn to_safe_html(markdown: &str) -> String {
    let mut options = Options::default();
    options.extension.strikethrough = true;
    options.extension.table = true;
    options.extension.autolink = true;
    options.extension.tasklist = true;
    options.extension.header_id_prefix = Some(String::new());
    // ammonia sanitizes the result, so raw HTML can be let through here.
    options.render.r#unsafe = true;

    let html = sanitizer()
        .clean(&markdown_to_html(markdown, &options))
        .to_string();
    // Images in posts load when they're about to scroll into view.
    let html = html.replace("<img ", r#"<img loading="lazy" decoding="async" "#);
    without_heading_links(&html)
}

/// comrak gives each heading an id and adds an empty link to it at the end
/// (`<h2 id="costs">Costs<a href="#costs" class="anchor"></a></h2>`). The
/// id stays, so `#costs` links still work; the empty link goes, because
/// screen readers can't name it.
fn without_heading_links(html: &str) -> String {
    let mut out = String::with_capacity(html.len());
    let mut rest = html;
    while let Some(start) = rest.find("<a href=\"#") {
        let Some(tag_end) = rest[start..].find('>') else {
            break;
        };
        let tag = &rest[start..start + tag_end + 1];
        let after = &rest[start + tag.len()..];
        let closes_heading = (1..=6).any(|n| after.starts_with(&format!("</a></h{n}>")));
        out.push_str(&rest[..start]);
        if tag.contains("class=\"anchor\"") && closes_heading {
            rest = &after["</a>".len()..];
        } else {
            out.push_str(tag);
            rest = after;
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
    let mut options = Options::default();
    options.extension.strikethrough = true;
    options.extension.table = true;
    options.extension.autolink = true;
    let root = comrak::parse_document(&arena, markdown, &options);

    let mut text = String::new();
    for node in root.descendants() {
        match &node.data.borrow().value {
            NodeValue::Text(t) => text.push_str(t),
            NodeValue::Code(c) => text.push_str(&c.literal),
            // A post written in HTML (pasted, or imported from WordPress):
            // its text, without the tags, is still the post's text.
            NodeValue::HtmlBlock(block) => {
                text.push_str(&html_text(&block.literal));
                text.push(' ');
            }
            NodeValue::SoftBreak | NodeValue::LineBreak => text.push(' '),
            // A new block: end the one before as a sentence, so headings and
            // list items don't run into the next text ("The plan. Keep it small.").
            NodeValue::Paragraph | NodeValue::Heading(_) | NodeValue::Item(_)
                if !text.trim_end().is_empty() =>
            {
                text.truncate(text.trim_end().len());
                if !text.ends_with(['.', '!', '?', ':', ';', '…']) {
                    text.push('.');
                }
                text.push(' ');
            }
            _ => {}
        }
    }
    text.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// The text inside a piece of HTML, without tags, scripts or styles.
fn html_text(html: &str) -> String {
    // ammonia removes every tag (and what's inside <script> and <style>) and
    // gives back HTML-escaped text; the excerpt wants it plain again.
    let escaped = ammonia::Builder::empty().clean(html).to_string();
    escaped
        .replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&quot;", "\"")
        .replace("&#39;", "'")
        .replace("&nbsp;", " ")
        .replace("&amp;", "&")
}

/// Plain-text excerpt of a Markdown document, cut at a word boundary.
pub fn excerpt(markdown: &str, max_chars: usize) -> String {
    let text = super::shortcodes::strip(&to_plain_text(beginning(markdown, max_chars * 20)));
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

/// The start of a long document, enough for an excerpt: about `limit`
/// characters, cut at a paragraph break when there's one. Lists of posts
/// show many excerpts, and parsing whole long posts for each was slow.
fn beginning(markdown: &str, limit: usize) -> &str {
    let Some((end, _)) = markdown.char_indices().nth(limit) else {
        return markdown; // Short enough already.
    };
    let head = &markdown[..end];
    match head.rfind("\n\n") {
        Some(i) if i > limit / 2 => &head[..i],
        _ => head,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn excerpts_of_long_posts_only_read_the_beginning() {
        let section = "A first paragraph with **bold** words. ".repeat(10)
            + "\n\n"
            + &"Another paragraph that goes on. ".repeat(20)
            + "\n\n";
        let long = section.repeat(200);
        assert_eq!(excerpt(&long, 200), {
            let text = crate::content::shortcodes::strip(&to_plain_text(&long));
            let cut: String = text.chars().take(200).collect();
            let cut = &cut[..cut.rfind(' ').unwrap()];
            format!(
                "{}…",
                cut.trim_end_matches(|c: char| c.is_ascii_punctuation())
            )
        });
        assert_eq!(beginning("short", 100), "short");
    }

    #[test]
    fn excerpts_of_posts_written_in_html_keep_their_text() {
        let html = "<p>Fish &amp; chips <em>by</em> the sea.</p>\n<script>alert(1)</script>";
        assert_eq!(excerpt(html, 200), "Fish & chips by the sea.");
    }

    #[test]
    fn excerpt_strips_markup_and_cuts_on_words() {
        assert_eq!(
            excerpt("# Title\n\nSome **bold** `code`.", 100),
            "Title. Some bold code."
        );
        assert_eq!(excerpt("one two three four", 9), "one two…");
        assert_eq!(
            excerpt("Is it fast?\n\n- One binary\n- One database", 100),
            "Is it fast? One binary. One database"
        );
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
