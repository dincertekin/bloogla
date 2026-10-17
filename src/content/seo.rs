//! The SEO block every public page puts in `<head>`: description, canonical
//! link, Open Graph and Twitter cards, feed links and JSON-LD structured data.

use super::text::{escape_html, to_iso8601};

use serde_json::json;

/// What a public page represents, for Open Graph and JSON-LD.
pub enum SeoKind<'a> {
    Website,
    Post {
        published_at: &'a str,
        tags: Vec<&'a str>,
        /// The writer, when it differs from the site's publisher.
        author: Option<&'a str>,
    },
}

/// Per-page metadata rendered into `<head>` by [`render_head`].
pub struct SeoMeta<'a> {
    pub title: &'a str,
    pub description: &'a str,
    /// Absolute URL of the canonical version of this page.
    pub canonical_url: &'a str,
    /// Image URL; relative paths are resolved against the base URL.
    pub image: Option<&'a str>,
    pub kind: SeoKind<'a>,
    pub noindex: bool,
}

/// Site-wide values shared by every page.
pub struct SiteInfo<'a> {
    pub name: &'a str,
    pub base_url: &'a str,
    pub publisher_type: &'a str,
    pub publisher_name: &'a str,
    /// Favicon URL, if one is set.
    pub icon: Option<&'a str>,
}

/// Build the SEO `<head>` block: description, canonical link, Open Graph,
/// Twitter card, RSS discovery link and JSON-LD structured data.
///
/// Themes include it with `{{ seo_head | safe }}`.
pub fn render_head(meta: &SeoMeta, site: &SiteInfo) -> String {
    let image = meta.image.map(|img| absolute_url(img, site.base_url));
    let og_type = match meta.kind {
        SeoKind::Website => "website",
        SeoKind::Post { .. } => "article",
    };

    let mut head = String::new();
    let mut tag = |line: String| {
        head.push_str(&line);
        head.push('\n');
    };

    if !meta.description.is_empty() {
        tag(format!(
            r#"<meta name="description" content="{}">"#,
            escape_html(meta.description)
        ));
    }
    if let Some(icon) = site.icon {
        tag(format!(r#"<link rel="icon" href="{}">"#, escape_html(icon)));
        tag(format!(
            r#"<link rel="apple-touch-icon" href="{}">"#,
            escape_html(icon)
        ));
    } else {
        // No icon set: an empty one, so browsers don't request /favicon.ico
        // and log an error.
        tag(r#"<link rel="icon" href="data:,">"#.to_string());
    }
    if meta.noindex {
        tag(r#"<meta name="robots" content="noindex">"#.to_string());
    }
    tag(format!(
        r#"<link rel="canonical" href="{}">"#,
        escape_html(meta.canonical_url)
    ));
    tag(format!(
        r#"<link rel="alternate" type="application/rss+xml" title="{}" href="{}/rss.xml">"#,
        escape_html(site.name),
        escape_html(site.base_url)
    ));

    for (property, value) in [
        ("og:site_name", site.name),
        ("og:type", og_type),
        ("og:title", meta.title),
        ("og:description", meta.description),
        ("og:url", meta.canonical_url),
    ] {
        if !value.is_empty() {
            tag(format!(
                r#"<meta property="{property}" content="{}">"#,
                escape_html(value)
            ));
        }
    }
    if let Some(ref img) = image {
        tag(format!(
            r#"<meta property="og:image" content="{}">"#,
            escape_html(img)
        ));
    }
    if let SeoKind::Post {
        published_at,
        ref tags,
        ..
    } = meta.kind
    {
        tag(format!(
            r#"<meta property="article:published_time" content="{}">"#,
            escape_html(&to_iso8601(published_at))
        ));
        for t in tags {
            tag(format!(
                r#"<meta property="article:tag" content="{}">"#,
                escape_html(t)
            ));
        }
    }
    tag(format!(
        r#"<meta name="twitter:card" content="{}">"#,
        if image.is_some() {
            "summary_large_image"
        } else {
            "summary"
        }
    ));

    let publisher_type = match site.publisher_type {
        "Organization" => "Organization",
        _ => "Person",
    };
    let publisher = json!({ "@type": publisher_type, "name": site.publisher_name });
    let image_json = image
        .as_deref()
        .map_or(serde_json::Value::Null, |i| json!(i));
    let structured = match meta.kind {
        SeoKind::Website => json!({
            "@context": "https://schema.org",
            "@type": "WebSite",
            "name": site.name,
            "url": site.base_url,
            "description": meta.description,
            "publisher": publisher,
        }),
        SeoKind::Post {
            published_at,
            ref tags,
            author,
        } => json!({
            "@context": "https://schema.org",
            "@type": "BlogPosting",
            "headline": meta.title,
            "description": meta.description,
            "url": meta.canonical_url,
            "mainEntityOfPage": meta.canonical_url,
            "datePublished": to_iso8601(published_at),
            "image": image_json,
            "keywords": tags.join(", "),
            "author": author.map_or(publisher.clone(), |name| json!({ "@type": "Person", "name": name })),
            "publisher": publisher,
        }),
    };
    let structured = without_empty_fields(structured);
    // `</` inside a JSON string could close the <script> element early.
    let structured = structured.to_string().replace("</", "<\\/");
    tag(format!(
        r#"<script type="application/ld+json">{structured}</script>"#
    ));

    head
}

/// Drop `null` and empty-string fields so structured data only states what is known.
fn without_empty_fields(value: serde_json::Value) -> serde_json::Value {
    match value {
        serde_json::Value::Object(map) => serde_json::Value::Object(
            map.into_iter()
                .filter(|(_, v)| !v.is_null() && v.as_str() != Some(""))
                .collect(),
        ),
        other => other,
    }
}

/// Resolve a root-relative URL (e.g. `/uploads/a.jpg`) against the site's base URL.
pub fn absolute_url(url: &str, base_url: &str) -> String {
    if url.starts_with('/') && !url.starts_with("//") {
        format!("{base_url}{url}")
    } else {
        url.to_string()
    }
}

// ---- Code the owner adds to every page (Settings → Site) ----

/// The longest head code kept.
const MAX_HEAD_CODE: usize = 5000;

/// Shown when the head code has something other than allowed tags.
pub const HEAD_CODE_ONLY_TAGS: &str = "Only meta and link tags can go here. Scripts and other code would slow your site down and could track your visitors.";
/// Shown for a link that would load something from another site.
pub const HEAD_CODE_LINK: &str = "This link tag would load something from another site, so it can't be added. Links with rel=\"me\", \"author\" or \"alternate\" and an https:// address are fine.";
/// Shown for a meta tag that changes how pages behave.
pub const HEAD_CODE_HTTP_EQUIV: &str =
    "A meta tag with http-equiv changes how your site works, so it can't be added.";

/// Check the tags someone pasted for every page, and give them back written
/// out cleanly, one per line. Only `<meta>` and some `<link>` tags are kept:
/// the ones services ask for to prove a site is yours (Google, Bing,
/// Pinterest, Mastodon). Nothing that runs code or loads from other sites.
pub fn check_head_code(input: &str) -> Result<String, &'static str> {
    let mut rest = input.trim();
    if rest.len() > MAX_HEAD_CODE {
        return Err(HEAD_CODE_ONLY_TAGS);
    }
    let mut tags = Vec::new();
    while !rest.is_empty() {
        let (name, attributes, after) = read_tag(rest).ok_or(HEAD_CODE_ONLY_TAGS)?;
        let value = |wanted: &str| {
            attributes
                .iter()
                .find(|(name, _)| name == wanted)
                .map(|(_, value)| value.as_str())
        };
        match name.as_str() {
            "meta" if value("http-equiv").is_some() => return Err(HEAD_CODE_HTTP_EQUIV),
            "meta" => {}
            "link" => {
                let rel = value("rel").unwrap_or_default().to_lowercase();
                let fine_rel = rel
                    .split_whitespace()
                    .all(|r| matches!(r, "me" | "author" | "alternate"));
                let href = value("href").unwrap_or_default();
                if rel.is_empty() || !fine_rel || !href.starts_with("https://") {
                    return Err(HEAD_CODE_LINK);
                }
            }
            _ => return Err(HEAD_CODE_ONLY_TAGS),
        }
        // Events like onload="..." would run code.
        if attributes.iter().any(|(name, _)| name.starts_with("on")) {
            return Err(HEAD_CODE_ONLY_TAGS);
        }
        let written: String = attributes
            .iter()
            .map(|(name, value)| format!(" {name}=\"{}\"", escape_html(value)))
            .collect();
        tags.push(format!("<{name}{written}>"));
        rest = after.trim_start();
    }
    Ok(tags.join("\n"))
}

/// A tag's attributes: `(name, value)`, names in lowercase.
type Attributes = Vec<(String, String)>;

/// Read one tag at the start of `text`: its name, its attributes and the
/// text after it. `None` if it isn't a tag.
fn read_tag(text: &str) -> Option<(String, Attributes, &str)> {
    let inside = text.strip_prefix('<')?;
    let name_end = inside
        .find(|c: char| !c.is_ascii_alphanumeric())
        .unwrap_or(inside.len());
    let name = inside[..name_end].to_ascii_lowercase();
    if name.is_empty() {
        return None;
    }
    let mut rest = &inside[name_end..];
    let mut attributes = Vec::new();
    loop {
        rest = rest.trim_start();
        if let Some(after) = rest.strip_prefix("/>").or_else(|| rest.strip_prefix('>')) {
            return Some((name, attributes, after));
        }
        let attribute_end =
            rest.find(|c: char| c.is_whitespace() || matches!(c, '=' | '>' | '/'))?;
        let attribute = rest[..attribute_end].to_ascii_lowercase();
        if attribute.is_empty()
            || !attribute
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | ':' | '_'))
        {
            return None;
        }
        rest = rest[attribute_end..].trim_start();
        let mut value = String::new();
        if let Some(after_equals) = rest.strip_prefix('=') {
            let after_equals = after_equals.trim_start();
            let quote = after_equals.chars().next()?;
            if quote == '"' || quote == '\'' {
                let end = after_equals[1..].find(quote)?;
                value = after_equals[1..=end].to_string();
                rest = &after_equals[end + 2..];
            } else {
                let end = after_equals
                    .find(|c: char| c.is_whitespace() || c == '>')
                    .unwrap_or(after_equals.len());
                value = after_equals[..end].to_string();
                rest = &after_equals[end..];
            }
        }
        attributes.push((attribute, value));
    }
}

#[cfg(test)]
mod head_code_tests {
    use super::*;

    #[test]
    fn keeps_verification_tags_written_cleanly() {
        let pasted = "  <meta name=\"google-site-verification\" content=\"x7Gm\" />\n<META name='msvalidate.01' content=ABC>\n<link rel=\"me\" href=\"https://mastodon.social/@ayse\">  ";
        assert_eq!(
            check_head_code(pasted).unwrap(),
            "<meta name=\"google-site-verification\" content=\"x7Gm\">\n<meta name=\"msvalidate.01\" content=\"ABC\">\n<link rel=\"me\" href=\"https://mastodon.social/@ayse\">"
        );
        assert_eq!(check_head_code("   ").unwrap(), "");
    }

    #[test]
    fn refuses_anything_that_runs_or_loads() {
        for pasted in [
            "<script>alert(1)</script>",
            "<script src=\"https://tracker.example/t.js\"></script>",
            "<style>body{display:none}</style>",
            "Some text",
            "<meta name=\"a\" content=\"b\" onload=\"alert(1)\">",
            "<meta name=\"a\" content=\"b\"><img src=x onerror=alert(1)>",
            "<meta name=\"a\" content=\"unclosed",
        ] {
            assert_eq!(
                check_head_code(pasted),
                Err(HEAD_CODE_ONLY_TAGS),
                "{pasted}"
            );
        }
        for pasted in [
            "<link rel=\"stylesheet\" href=\"https://cdn.example/a.css\">",
            "<link rel=\"me\" href=\"javascript:alert(1)\">",
            "<link rel=\"preload\" href=\"https://a.example/font.woff2\">",
        ] {
            assert_eq!(check_head_code(pasted), Err(HEAD_CODE_LINK), "{pasted}");
        }
        assert_eq!(
            check_head_code("<meta http-equiv=\"refresh\" content=\"0;url=https://evil.example\">"),
            Err(HEAD_CODE_HTTP_EQUIV)
        );
        // A quote in a value can't break out of the attribute.
        assert_eq!(
            check_head_code("<meta name='a' content='x\"><script>'>").unwrap(),
            "<meta name=\"a\" content=\"x&quot;&gt;&lt;script&gt;\">"
        );
    }
}
