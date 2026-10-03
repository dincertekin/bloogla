//! Shortcodes: `[name args]` in post text, expanded after Markdown is rendered
//! and sanitized.
//!
//! Built in: `[youtube URL]`, `[vimeo URL]`, `[audio URL]`, `[video URL]`, `[toc]`.
//! Themes add their own with `themes/<theme>/shortcodes/<name>.html`; the
//! template receives `args` (positional values) and `params` (`key=value`).
//! Shortcodes inside `<code>`/`<pre>` stay as text, and `[[name]]` prints `[name]`.

use std::collections::HashMap;

/// Parsed shortcode arguments: `[name first second key="value"]`.
#[derive(Debug, Default, serde::Serialize)]
pub struct Args {
    pub args: Vec<String>,
    pub params: HashMap<String, String>,
}

impl Args {
    fn first(&self) -> Option<&str> {
        self.args.first().map(String::as_str)
    }
}

/// Expand shortcodes in sanitized HTML. `custom` renders theme shortcodes and
/// returns `None` for names it doesn't know (left untouched).
pub fn expand(html: &str, custom: &dyn Fn(&str, &Args) -> Option<String>) -> String {
    let mut out = String::with_capacity(html.len());
    let mut rest = html;
    let mut code_depth = 0usize;

    while let Some(pos) = rest.find(['[', '<']) {
        out.push_str(&rest[..pos]);
        rest = &rest[pos..];

        if rest.starts_with('<') {
            // Copy tags through, tracking whether we're inside code.
            let end = rest.find('>').map_or(rest.len(), |i| i + 1);
            let tag = &rest[..end];
            let lower = tag.to_ascii_lowercase();
            if lower.starts_with("<pre") || lower.starts_with("<code") {
                code_depth += 1;
            } else if lower.starts_with("</pre") || lower.starts_with("</code") {
                code_depth = code_depth.saturating_sub(1);
            }
            out.push_str(tag);
            rest = &rest[end..];
            continue;
        }

        // `[[name]]` is an escaped shortcode.
        if let Some(inner) = rest.strip_prefix("[[") {
            if let Some(close) = inner.find("]]") {
                if code_depth == 0 && is_name_start(&inner[..close]) {
                    out.push('[');
                    out.push_str(&inner[..close]);
                    out.push(']');
                    rest = &inner[close + 2..];
                    continue;
                }
            }
        }

        let Some(close) = rest.find(']') else {
            out.push_str(rest);
            rest = "";
            break;
        };
        let body = &rest[1..close];
        let expanded = (code_depth == 0 && is_name_start(body))
            .then(|| {
                let (name, raw_args) = body.split_once(' ').unwrap_or((body, ""));
                // Markdown may have turned a URL argument into a link; use its text.
                let args = parse_args(&decode_entities(&strip_tags(raw_args)));
                builtin(name, &args, html).or_else(|| custom(name, &args))
            })
            .flatten();

        match expanded {
            Some(replacement) => {
                let mut after = &rest[close + 1..];
                // An autolinked URL can swallow the closing `]` into its link;
                // drop that link's end tag along with the shortcode.
                if body.contains("<a ") && !body.contains("</a>") {
                    after = after.strip_prefix("</a>").unwrap_or(after);
                }
                // A shortcode alone in a paragraph replaces the paragraph.
                if out.ends_with("<p>") && after.starts_with("</p>") {
                    out.truncate(out.len() - 3);
                    after = &after[4..];
                }
                out.push_str(&replacement);
                rest = after;
            }
            None => {
                out.push('[');
                rest = &rest[1..];
            }
        }
    }
    out.push_str(rest);
    out
}

/// Remove shortcodes from plain text (for excerpts), keeping quoted text like
/// the message of `[note "..."]`.
pub fn strip(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut rest = text;
    while let Some(open) = rest.find('[') {
        out.push_str(&rest[..open]);
        let after = &rest[open + 1..];
        match after.find(']') {
            Some(close) if is_name_start(&after[..close]) => {
                let args = parse_args(after[..close].split_once(' ').map_or("", |(_, a)| a));
                for quoted in args.args.iter().filter(|a| a.contains(' ')) {
                    out.push_str(quoted);
                    out.push(' ');
                }
                rest = &after[close + 1..];
            }
            _ => {
                out.push('[');
                rest = after;
            }
        }
    }
    out.push_str(rest);
    out.split_whitespace().collect::<Vec<_>>().join(" ")
}

fn is_name_start(body: &str) -> bool {
    let name = body.split(' ').next().unwrap_or_default();
    !name.is_empty()
        && name.len() <= 32
        && name.starts_with(|c: char| c.is_ascii_lowercase())
        && name
            .chars()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_' || c == '-')
}

/// Split `a "b c" key=value key2="x y"` into positional args and params.
fn parse_args(raw: &str) -> Args {
    let mut args = Args::default();
    let mut chars = raw.trim().chars().peekable();
    while chars.peek().is_some() {
        while chars.peek().is_some_and(|c| c.is_whitespace()) {
            chars.next();
        }
        let mut token = String::new();
        let mut key: Option<String> = None;
        while let Some(&c) = chars.peek() {
            if c.is_whitespace() {
                break;
            }
            chars.next();
            match c {
                '"' => {
                    for q in chars.by_ref() {
                        if q == '"' {
                            break;
                        }
                        token.push(q);
                    }
                }
                // `key=value` only when the key is a plain word, so URLs like
                // `watch?v=abc` stay whole.
                '=' if key.is_none() && is_param_name(&token) => {
                    key = Some(std::mem::take(&mut token))
                }
                _ => token.push(c),
            }
        }
        match key {
            Some(k) => {
                args.params.insert(k, token);
            }
            None if !token.is_empty() => args.args.push(token),
            None => {}
        }
    }
    args
}

fn is_param_name(token: &str) -> bool {
    token.starts_with(|c: char| c.is_ascii_alphabetic())
        && token
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-')
}

/// Undo the HTML escaping Markdown applied to text.
fn decode_entities(text: &str) -> String {
    text.replace("&quot;", "\"")
        .replace("&#39;", "'")
        .replace("&#x27;", "'")
        .replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&amp;", "&")
}

fn escape(text: &str) -> String {
    super::text::escape_html(text)
}

fn builtin(name: &str, args: &Args, html: &str) -> Option<String> {
    match name {
        "youtube" => youtube_id(args.first()?).map(|id| {
            format!(
                r#"<div class="embed"><iframe src="https://www.youtube-nocookie.com/embed/{id}" title="YouTube video" loading="lazy" allow="accelerometer; encrypted-media; gyroscope; picture-in-picture; fullscreen" referrerpolicy="strict-origin-when-cross-origin" allowfullscreen></iframe></div>"#
            )
        }),
        "vimeo" => {
            let id: String = args
                .first()?
                .rsplit('/')
                .next()?
                .chars()
                .take_while(char::is_ascii_digit)
                .collect();
            (!id.is_empty()).then(|| {
                format!(
                    r#"<div class="embed"><iframe src="https://player.vimeo.com/video/{id}?dnt=1" title="Vimeo video" loading="lazy" allow="fullscreen; picture-in-picture" allowfullscreen></iframe></div>"#
                )
            })
        }
        "audio" | "video" => {
            let url = args.first().filter(|u| is_media_url(u))?;
            Some(format!(
                r#"<{name} class="media" controls preload="metadata" src="{}"></{name}>"#,
                escape(url)
            ))
        }
        "toc" => Some(table_of_contents(html)),
        _ => None,
    }
}

fn is_media_url(url: &str) -> bool {
    (url.starts_with('/') && !url.starts_with("//")) || url.starts_with("https://")
}

/// YouTube video id from a watch, short, embed or youtu.be URL, or a bare id.
fn youtube_id(input: &str) -> Option<String> {
    let candidate = if let Some((_, query)) = input.split_once("v=") {
        query.split('&').next().unwrap_or_default()
    } else {
        input
            .trim_end_matches('/')
            .rsplit('/')
            .next()
            .unwrap_or_default()
            .split('?')
            .next()
            .unwrap_or_default()
    };
    (candidate.len() == 11
        && candidate
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_'))
    .then(|| candidate.to_string())
}

/// A linked list of the `<h2>`/`<h3>` headings that have ids.
fn table_of_contents(html: &str) -> String {
    let mut items = String::new();
    let mut rest = html;
    while let Some(start) = rest.find("<h") {
        rest = &rest[start..];
        let level = rest.as_bytes().get(2).copied();
        let Some(level @ (b'2' | b'3')) = level else {
            rest = &rest[2..];
            continue;
        };
        let close_tag = format!("</h{}>", level as char);
        let Some(end) = rest.find(&close_tag) else {
            break;
        };
        let heading = &rest[..end];
        rest = &rest[end + close_tag.len()..];

        let Some(id) = heading
            .split("id=\"")
            .nth(1)
            .and_then(|s| s.split('"').next())
        else {
            continue;
        };
        let text = strip_tags(heading);
        if text.is_empty() {
            continue;
        }
        let class = if level == b'3' {
            r#" class="toc-sub""#
        } else {
            ""
        };
        items.push_str(&format!(r##"<li{class}><a href="#{id}">{text}</a></li>"##));
    }
    if items.is_empty() {
        return String::new();
    }
    format!(r#"<nav class="toc" aria-label="Contents"><ul>{items}</ul></nav>"#)
}

fn strip_tags(html: &str) -> String {
    let mut text = String::new();
    let mut in_tag = false;
    for c in html.chars() {
        match c {
            '<' => in_tag = true,
            '>' => in_tag = false,
            _ if !in_tag => text.push(c),
            _ => {}
        }
    }
    text.trim().to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn none(_: &str, _: &Args) -> Option<String> {
        None
    }

    #[test]
    fn youtube_from_any_url() {
        for url in [
            "https://www.youtube.com/watch?v=dQw4w9WgXcQ&t=10",
            "https://youtu.be/dQw4w9WgXcQ",
            "https://www.youtube.com/shorts/dQw4w9WgXcQ",
            "dQw4w9WgXcQ",
        ] {
            assert_eq!(youtube_id(url).as_deref(), Some("dQw4w9WgXcQ"), "{url}");
        }
        assert_eq!(youtube_id("javascript:alert(1)"), None);
    }

    #[test]
    fn replaces_whole_paragraph_and_keeps_code() {
        let html = "<p>[youtube https://youtu.be/dQw4w9WgXcQ]</p><p><code>[youtube x]</code> and [[toc]]</p>";
        let out = expand(html, &none);
        assert!(out.starts_with(
            r#"<div class="embed"><iframe src="https://www.youtube-nocookie.com/embed/dQw4w9WgXcQ""#
        ));
        assert!(out.contains("<code>[youtube x]</code> and [toc]</p>"));
        assert!(!out.contains("<p><div"));
    }

    #[test]
    fn strip_removes_shortcodes_from_excerpts() {
        assert_eq!(
            strip(
                r#"[toc] Getting there [youtube https://youtu.be/x] [note "Buy a card." kind=warning] Done [x"#
            ),
            "Getting there Buy a card. Done [x"
        );
    }

    #[test]
    fn autolinked_url_arguments_still_work() {
        let html = r#"<p>[youtube <a href="https://youtu.be/dQw4w9WgXcQ">https://youtu.be/dQw4w9WgXcQ</a>]</p>"#;
        assert!(expand(html, &none).contains("youtube-nocookie.com/embed/dQw4w9WgXcQ"));
    }

    #[test]
    fn autolink_that_swallowed_the_bracket() {
        let html = r#"<p>[youtube <a href="https://www.youtube.com/watch?v=dQw4w9WgXcQ%5D" rel="noopener noreferrer">https://www.youtube.com/watch?v=dQw4w9WgXcQ]</a></p>"#;
        let out = expand(html, &none);
        assert_eq!(
            out,
            r#"<div class="embed"><iframe src="https://www.youtube-nocookie.com/embed/dQw4w9WgXcQ" title="YouTube video" loading="lazy" allow="accelerometer; encrypted-media; gyroscope; picture-in-picture; fullscreen" referrerpolicy="strict-origin-when-cross-origin" allowfullscreen></iframe></div>"#
        );
    }

    #[test]
    fn leaves_unknown_and_unsafe_alone() {
        let html = "<p>[not a shortcode] [audio javascript:alert(1)] [link](x)</p>";
        assert_eq!(expand(html, &none), html);
    }

    #[test]
    fn parses_quoted_args_and_params() {
        let args = parse_args(r#"one "two three" kind="tip" n=5 https://x.com/?v=1"#);
        assert_eq!(args.args, ["one", "two three", "https://x.com/?v=1"]);
        assert_eq!(args.params["kind"], "tip");
        assert_eq!(args.params["n"], "5");
    }

    #[test]
    fn toc_lists_headings() {
        let html = r##"[toc]<h2><a href="#a" class="anchor" id="a"></a>First</h2><p>x</p><h3><a id="b"></a>Sub</h3>"##;
        let out = expand(html, &none);
        assert!(out.starts_with(r##"<nav class="toc" aria-label="Contents"><ul><li><a href="#a">First</a></li><li class="toc-sub"><a href="#b">Sub</a></li></ul></nav>"##));
    }

    #[test]
    fn custom_shortcodes_get_args() {
        let custom = |name: &str, args: &Args| {
            (name == "note").then(|| format!("<aside>{}</aside>", args.args[0]))
        };
        assert_eq!(
            expand("<p>[note &quot;Hi there&quot;]</p>", &custom),
            "<aside>Hi there</aside>"
        );
    }
}
