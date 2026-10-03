//! Import posts and pages from a WordPress export file (WXR, Tools → Export).

use crate::content::text::{normalize_datetime, percent_decode, slugify};

use quick_xml::escape::resolve_predefined_entity;
use quick_xml::events::{BytesStart, Event};
use quick_xml::{Reader, XmlVersion};
use sqlx::SqlitePool;
use std::collections::HashMap;

#[derive(Default)]
struct WpItem {
    title: String,
    slug: String,
    post_id: String,
    post_type: String,
    status: String,
    date: String,
    date_gmt: String,
    content: String,
    attachment_url: String,
    thumbnail_id: Option<String>,
    /// `(domain, nicename, display name)`; domain is `category` or `post_tag`.
    terms: Vec<(String, String, String)>,
}

#[derive(Debug, Default)]
pub struct ImportReport {
    pub posts: usize,
    pub pages: usize,
    pub skipped_existing: Vec<String>,
    pub skipped_other: usize,
    pub tags_created: usize,
}

/// Import a WXR file into the database.
pub async fn import(pool: &SqlitePool, xml: &str) -> Result<ImportReport, String> {
    let items = parse_wxr(xml)?;

    let attachments: HashMap<&str, &str> = items
        .iter()
        .filter(|i| i.post_type == "attachment" && !i.attachment_url.is_empty())
        .map(|i| (i.post_id.as_str(), i.attachment_url.as_str()))
        .collect();

    let mut report = ImportReport::default();
    let db_error = |e: sqlx::Error| format!("Database error: {e}");

    for item in &items {
        let is_page = match item.post_type.as_str() {
            "post" => false,
            "page" => true,
            _ => {
                if item.post_type != "attachment" {
                    report.skipped_other += 1;
                }
                continue;
            }
        };
        let status = match item.status.as_str() {
            "publish" => "published",
            "future" => "scheduled",
            "draft" | "pending" | "private" => "draft",
            _ => {
                // trash, auto-draft, inherit...
                report.skipped_other += 1;
                continue;
            }
        };

        let title = if item.title.trim().is_empty() {
            "Untitled"
        } else {
            item.title.trim()
        };
        let slug_source = percent_decode(&item.slug);
        let slug = slugify(if slug_source.trim().is_empty() {
            title
        } else {
            &slug_source
        });

        let exists: Option<i64> = sqlx::query_scalar("SELECT id FROM posts WHERE slug = ?")
            .bind(&slug)
            .fetch_optional(pool)
            .await
            .map_err(db_error)?;
        if exists.is_some() {
            report.skipped_existing.push(slug);
            continue;
        }

        let date = if item.date_gmt.is_empty() || item.date_gmt.starts_with("0000") {
            &item.date
        } else {
            &item.date_gmt
        };
        let published_at = normalize_datetime(Some(date));
        let cover = item
            .thumbnail_id
            .as_deref()
            .and_then(|id| attachments.get(id).copied());

        let post_id = sqlx::query(
            "INSERT INTO posts (title, slug, content, cover_image, status, published_at, is_page)
             VALUES (?, ?, ?, ?, ?, ?, ?)",
        )
        .bind(title)
        .bind(&slug)
        .bind(strip_block_comments(&item.content))
        .bind(cover)
        .bind(status)
        .bind(&published_at)
        .bind(is_page)
        .execute(pool)
        .await
        .map_err(db_error)?
        .last_insert_rowid();

        for (domain, nicename, name) in &item.terms {
            if domain != "category" && domain != "post_tag" {
                continue;
            }
            let tag_slug = slugify(&percent_decode(if nicename.is_empty() {
                name
            } else {
                nicename
            }));
            if tag_slug == "uncategorized" {
                continue;
            }

            let tag_id: Option<i64> = sqlx::query_scalar("SELECT id FROM tags WHERE slug = ?")
                .bind(&tag_slug)
                .fetch_optional(pool)
                .await
                .map_err(db_error)?;
            let tag_id = match tag_id {
                Some(id) => id,
                None => {
                    report.tags_created += 1;
                    sqlx::query("INSERT INTO tags (name, slug) VALUES (?, ?)")
                        .bind(name.trim())
                        .bind(&tag_slug)
                        .execute(pool)
                        .await
                        .map_err(db_error)?
                        .last_insert_rowid()
                }
            };

            sqlx::query("INSERT OR IGNORE INTO post_tags (post_id, tag_id) VALUES (?, ?)")
                .bind(post_id)
                .bind(tag_id)
                .execute(pool)
                .await
                .map_err(db_error)?;
        }

        if is_page {
            report.pages += 1;
        } else {
            report.posts += 1;
        }
    }

    Ok(report)
}

/// Collect `<item>` elements from a WXR document.
fn parse_wxr(xml: &str) -> Result<Vec<WpItem>, String> {
    let mut reader = Reader::from_str(xml);
    let mut items = Vec::new();
    let mut current: Option<WpItem> = None;
    let mut text = String::new();
    let mut term: Option<(String, String)> = None;
    let (mut meta_key, mut meta_value) = (String::new(), String::new());

    loop {
        let event = reader
            .read_event()
            .map_err(|e| format!("Invalid XML at byte {}: {e}", reader.buffer_position()))?;

        match event {
            Event::Start(start) => {
                text.clear();
                match start.name().as_ref() {
                    b"item" => current = Some(WpItem::default()),
                    b"category" if current.is_some() => {
                        term = Some((attr(&start, b"domain"), attr(&start, b"nicename")));
                    }
                    _ => {}
                }
            }
            Event::Text(t) => {
                let decoded = t.decode().map_err(|e| format!("Invalid XML text: {e}"))?;
                text.push_str(&decoded);
            }
            // Entities such as `&amp;` and `&#8217;` arrive separately from the text.
            Event::GeneralRef(entity) => {
                let invalid = |e: String| format!("Invalid XML entity: {e}");
                if let Some(c) = entity
                    .resolve_char_ref()
                    .map_err(|e| invalid(e.to_string()))?
                {
                    text.push(c);
                } else {
                    let name = entity.decode().map_err(|e| invalid(e.to_string()))?;
                    match resolve_predefined_entity(&name) {
                        Some(value) => text.push_str(value),
                        // Unknown entity: keep it as written.
                        None => text.push_str(&format!("&{name};")),
                    }
                }
            }
            Event::CData(c) => text.push_str(&String::from_utf8_lossy(&c.into_inner())),
            Event::End(end) => {
                let Some(item) = current.as_mut() else {
                    continue;
                };
                let value = std::mem::take(&mut text);
                match end.name().as_ref() {
                    b"title" => item.title = value,
                    b"content:encoded" => item.content = value,
                    b"wp:post_name" => item.slug = value,
                    b"wp:post_id" => item.post_id = value.trim().to_string(),
                    b"wp:post_type" => item.post_type = value.trim().to_string(),
                    b"wp:status" => item.status = value.trim().to_string(),
                    b"wp:post_date" => item.date = value.trim().to_string(),
                    b"wp:post_date_gmt" => item.date_gmt = value.trim().to_string(),
                    b"wp:attachment_url" => item.attachment_url = value.trim().to_string(),
                    b"wp:meta_key" => meta_key = value,
                    b"wp:meta_value" => meta_value = value,
                    b"wp:postmeta" => {
                        if meta_key == "_thumbnail_id" {
                            item.thumbnail_id = Some(meta_value.trim().to_string());
                        }
                        meta_key.clear();
                        meta_value.clear();
                    }
                    b"category" => {
                        if let Some((domain, nicename)) = term.take() {
                            item.terms.push((domain, nicename, value));
                        }
                    }
                    b"item" => items.extend(current.take()),
                    _ => {}
                }
            }
            Event::Eof => break,
            _ => {}
        }
    }

    Ok(items)
}

fn attr(start: &BytesStart, name: &[u8]) -> String {
    start
        .attributes()
        .flatten()
        .find(|a| a.key.as_ref() == name)
        .and_then(|a| a.normalized_value(XmlVersion::default()).ok())
        .map(|v| v.into_owned())
        .unwrap_or_default()
}

/// Remove Gutenberg block markers (`<!-- wp:paragraph -->`) so the editor stays readable.
fn strip_block_comments(content: &str) -> String {
    let mut out = String::with_capacity(content.len());
    let mut rest = content;
    loop {
        let marker = [rest.find("<!-- wp:"), rest.find("<!-- /wp:")]
            .into_iter()
            .flatten()
            .min();
        let Some(start) = marker else { break };
        let Some(len) = rest[start..].find("-->") else {
            break;
        };
        out.push_str(&rest[..start]);
        rest = &rest[start + len + 3..];
    }
    out.push_str(rest);

    // Collapse the blank lines left behind by removed markers.
    let mut collapsed = String::with_capacity(out.len());
    let mut blank_run = 0;
    for line in out.lines() {
        if line.trim().is_empty() {
            blank_run += 1;
            if blank_run > 1 {
                continue;
            }
        } else {
            blank_run = 0;
        }
        collapsed.push_str(line);
        collapsed.push('\n');
    }
    collapsed.trim().to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_items_with_entities() {
        let xml = r#"<rss><channel><item>
            <title>Tom &amp; Jerry&#8217;s &lt;day&gt;</title>
            <category domain="post_tag" nicename="a&amp;b"><![CDATA[A & B]]></category>
            <wp:post_type>post</wp:post_type>
        </item></channel></rss>"#;
        let items = parse_wxr(xml).unwrap();
        assert_eq!(items.len(), 1);
        assert_eq!(items[0].title, "Tom & Jerry’s <day>");
        assert_eq!(items[0].post_type, "post");
        assert_eq!(
            items[0].terms,
            [(
                "post_tag".to_string(),
                "a&b".to_string(),
                "A & B".to_string()
            )]
        );
    }

    #[test]
    fn strips_gutenberg_markers() {
        let html = "<!-- wp:paragraph -->\n<p>Hi</p>\n<!-- /wp:paragraph -->\n\n<!-- wp:image {\"id\":1} -->\n<figure>x</figure>\n<!-- /wp:image -->";
        assert_eq!(
            strip_block_comments(html),
            "<p>Hi</p>\n\n<figure>x</figure>"
        );
    }
}
