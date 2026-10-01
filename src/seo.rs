use crate::utils::xml_escape;

use serde_json::json;

/// What a public page represents, for Open Graph and JSON-LD.
pub enum SeoKind<'a> {
    Website,
    Post {
        published_at: &'a str,
        tags: Vec<&'a str>,
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
            xml_escape(meta.description)
        ));
    }
    if let Some(icon) = site.icon {
        tag(format!(r#"<link rel="icon" href="{}">"#, xml_escape(icon)));
        tag(format!(
            r#"<link rel="apple-touch-icon" href="{}">"#,
            xml_escape(icon)
        ));
    }
    if meta.noindex {
        tag(r#"<meta name="robots" content="noindex">"#.to_string());
    }
    tag(format!(
        r#"<link rel="canonical" href="{}">"#,
        xml_escape(meta.canonical_url)
    ));
    tag(format!(
        r#"<link rel="alternate" type="application/rss+xml" title="{}" href="{}/rss.xml">"#,
        xml_escape(site.name),
        xml_escape(site.base_url)
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
                xml_escape(value)
            ));
        }
    }
    if let Some(ref img) = image {
        tag(format!(
            r#"<meta property="og:image" content="{}">"#,
            xml_escape(img)
        ));
    }
    if let SeoKind::Post {
        published_at,
        ref tags,
    } = meta.kind
    {
        tag(format!(
            r#"<meta property="article:published_time" content="{}">"#,
            xml_escape(&iso_date(published_at))
        ));
        for t in tags {
            tag(format!(
                r#"<meta property="article:tag" content="{}">"#,
                xml_escape(t)
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
        } => json!({
            "@context": "https://schema.org",
            "@type": "BlogPosting",
            "headline": meta.title,
            "description": meta.description,
            "url": meta.canonical_url,
            "mainEntityOfPage": meta.canonical_url,
            "datePublished": iso_date(published_at),
            "image": image_json,
            "keywords": tags.join(", "),
            "author": publisher,
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

/// Convert a stored `YYYY-MM-DD HH:MM:SS` (UTC) date into ISO 8601.
fn iso_date(raw_date: &str) -> String {
    format!("{}Z", raw_date.replacen(' ', "T", 1))
}
