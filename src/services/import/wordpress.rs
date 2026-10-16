//! Reading a WordPress export file (WordPress: Tools → Export, "All content").
//!
//! The file is XML ("WXR"): the site's name and address, its people, and one
//! `<item>` for every post, page and uploaded image, with their comments.
//! Only what Bloogla uses is read; everything else is skipped.

use quick_xml::events::Event;
use quick_xml::Reader;

/// What doesn't look like a WordPress export at all.
pub const NOT_WORDPRESS: &str =
    "The file is not a WordPress export. In WordPress, use Tools → Export.";
/// A WordPress export that is cut short or broken.
pub const DAMAGED: &str =
    "The file is damaged or incomplete. Export it from WordPress again and try once more.";

/// Everything read from an export file.
#[derive(Debug, Default)]
pub struct Export {
    pub site_title: String,
    /// The old site's address, like `https://ayseskitchen.com`.
    pub site_url: String,
    pub authors: Vec<Author>,
    pub items: Vec<Item>,
}

/// Someone who wrote on the old site.
#[derive(Debug, Default, Clone)]
pub struct Author {
    /// Their WordPress user name; posts name their author by it.
    pub login: String,
    pub email: String,
    pub name: String,
}

/// A post, page or uploaded image ("attachment").
#[derive(Debug, Default, Clone)]
pub struct Item {
    pub wordpress_id: i64,
    pub title: String,
    /// Its address on the old site.
    pub link: String,
    /// `post`, `page`, `attachment`, or others Bloogla skips.
    pub kind: String,
    /// `publish`, `draft`, `future`, `private`, `pending`, `trash`...
    pub status: String,
    /// The last part of its address, as WordPress stored it.
    pub slug: String,
    pub content: String,
    /// In UTC, like `2024-05-01 09:30:00` (`0000-00-00 00:00:00` for drafts).
    pub date_utc: String,
    /// In the old site's time zone.
    pub date_local: String,
    /// The author's WordPress user name.
    pub author_login: String,
    /// Category and tag names.
    pub tags: Vec<String>,
    /// The cover image: the WordPress id of an attachment item.
    pub cover_id: Option<i64>,
    /// For attachments: where the file is.
    pub attachment_url: String,
    pub comments: Vec<Comment>,
}

/// A reader's comment on a post.
#[derive(Debug, Default, Clone)]
pub struct Comment {
    pub author: String,
    pub email: String,
    pub date_utc: String,
    pub content: String,
    /// Approved by the old site (not waiting, not spam).
    pub approved: bool,
    /// Empty for a comment; `pingback` or `trackback` for links from other sites.
    pub kind: String,
}

/// Read an export file. Fails with [`NOT_WORDPRESS`] or [`DAMAGED`].
pub fn read(xml: &[u8]) -> Result<Export, &'static str> {
    let mut reader = Reader::from_reader(xml);
    let mut export = Export::default();
    let mut is_wordpress = false;

    // Where we are: the names of the open elements, outermost first.
    let mut open: Vec<String> = Vec::new();
    // The text of the element that is open now.
    let mut text = String::new();
    let mut author = Author::default();
    let mut item = Item::default();
    let mut comment = Comment::default();
    // A `<wp:postmeta>`: its key, then its value.
    let mut meta_key = String::new();
    // Inside `<category domain="category">` or `domain="post_tag"`.
    let mut in_tag = false;

    let mut buffer = Vec::new();
    loop {
        let event = reader.read_event_into(&mut buffer).map_err(|_| {
            if is_wordpress {
                DAMAGED
            } else {
                NOT_WORDPRESS
            }
        })?;
        match event {
            Event::Start(start) => {
                let name = String::from_utf8_lossy(start.name().as_ref()).into_owned();
                if name == "category" {
                    in_tag = start.attributes().flatten().any(|attribute| {
                        attribute.key.as_ref() == b"domain"
                            && matches!(attribute.value.as_ref(), b"category" | b"post_tag")
                    });
                }
                open.push(name);
                text.clear();
            }
            Event::Text(part) => text.push_str(&part.decode().map_err(|_| DAMAGED)?),
            Event::CData(part) => text.push_str(&String::from_utf8_lossy(&part.into_inner())),
            // `&amp;`, `&#233;`...: XML's way of writing characters.
            Event::GeneralRef(reference) => {
                if let Ok(Some(character)) = reference.resolve_char_ref() {
                    text.push(character);
                } else {
                    let name: &[u8] = &reference;
                    match name {
                        b"amp" => text.push('&'),
                        b"lt" => text.push('<'),
                        b"gt" => text.push('>'),
                        b"quot" => text.push('"'),
                        b"apos" => text.push('\''),
                        _ => {}
                    }
                }
            }
            Event::End(_) => {
                let Some(name) = open.pop() else {
                    return Err(DAMAGED);
                };
                let value = std::mem::take(&mut text);
                let inside = |parent: &str| open.iter().any(|n| n == parent);
                match name.as_str() {
                    "wp:wxr_version" => is_wordpress = true,

                    // The site, directly inside <channel>.
                    "title" if open.len() == 2 => export.site_title = value.trim().into(),
                    "link" if open.len() == 2 && export.site_url.is_empty() => {
                        export.site_url = value.trim().into()
                    }
                    "wp:base_blog_url" => export.site_url = value.trim().into(),

                    // People.
                    "wp:author_login" => author.login = value.trim().into(),
                    "wp:author_email" => author.email = value.trim().into(),
                    "wp:author_display_name" => author.name = value.trim().into(),
                    "wp:author" => export.authors.push(std::mem::take(&mut author)),

                    // A comment, inside an item.
                    "wp:comment_author" => comment.author = value.trim().into(),
                    "wp:comment_author_email" => comment.email = value.trim().into(),
                    "wp:comment_date_gmt" => comment.date_utc = value.trim().into(),
                    "wp:comment_content" => comment.content = value,
                    "wp:comment_approved" => comment.approved = value.trim() == "1",
                    "wp:comment_type" => comment.kind = value.trim().into(),
                    "wp:comment" => item.comments.push(std::mem::take(&mut comment)),

                    // Extra values on an item; only the cover image is used.
                    "wp:meta_key" => meta_key = value.trim().into(),
                    "wp:meta_value" if meta_key == "_thumbnail_id" => {
                        item.cover_id = value.trim().parse().ok()
                    }

                    // The item itself.
                    "title" if inside("item") => item.title = value.trim().into(),
                    "link" if inside("item") => item.link = value.trim().into(),
                    "wp:post_id" => item.wordpress_id = value.trim().parse().unwrap_or(0),
                    "wp:post_type" => item.kind = value.trim().into(),
                    "wp:status" => item.status = value.trim().into(),
                    "wp:post_name" => item.slug = value.trim().into(),
                    "content:encoded" => item.content = value,
                    "wp:post_date_gmt" => item.date_utc = value.trim().into(),
                    "wp:post_date" => item.date_local = value.trim().into(),
                    "dc:creator" => item.author_login = value.trim().into(),
                    "wp:attachment_url" => item.attachment_url = value.trim().into(),
                    "category" if in_tag => {
                        let tag = value.trim().to_string();
                        if !tag.is_empty() && !item.tags.contains(&tag) {
                            item.tags.push(tag);
                        }
                        in_tag = false;
                    }
                    "item" => export.items.push(std::mem::take(&mut item)),
                    _ => {}
                }
            }
            Event::Eof => break,
            _ => {}
        }
        buffer.clear();
    }

    if !is_wordpress {
        return Err(NOT_WORDPRESS);
    }
    if !open.is_empty() {
        return Err(DAMAGED);
    }
    Ok(export)
}

/// Remove the block editor's markers (`<!-- wp:paragraph -->`), which only
/// WordPress understands, so the post reads cleanly in Bloogla's editor.
pub fn without_block_markers(content: &str) -> String {
    let mut out = String::with_capacity(content.len());
    let mut rest = content;
    loop {
        let marker = [rest.find("<!-- wp:"), rest.find("<!-- /wp:")]
            .into_iter()
            .flatten()
            .min();
        let Some(start) = marker else { break };
        let Some(length) = rest[start..].find("-->") else {
            break;
        };
        out.push_str(&rest[..start]);
        rest = &rest[start + length + 3..];
    }
    out.push_str(rest);

    // Removed markers leave runs of blank lines; keep one.
    let mut tidy = String::with_capacity(out.len());
    let mut blank_lines = 0;
    for line in out.lines() {
        if line.trim().is_empty() {
            blank_lines += 1;
            if blank_lines > 1 {
                continue;
            }
        } else {
            blank_lines = 0;
        }
        tidy.push_str(line);
        tidy.push('\n');
    }
    tidy.trim().to_string()
}

#[cfg(test)]
pub mod tests {
    use super::*;

    /// A small export, written like WordPress writes it.
    pub const SAMPLE: &str = r#"<?xml version="1.0" encoding="UTF-8" ?>
<rss version="2.0" xmlns:content="http://purl.org/rss/1.0/modules/content/"
    xmlns:dc="http://purl.org/dc/elements/1.1/" xmlns:wp="http://wordpress.org/export/1.2/">
<channel>
    <title>Ayşe&#039;s Kitchen</title>
    <link>https://ayseskitchen.com</link>
    <wp:wxr_version>1.2</wp:wxr_version>
    <wp:base_blog_url>https://ayseskitchen.com</wp:base_blog_url>
    <wp:author><wp:author_login><![CDATA[ayse]]></wp:author_login><wp:author_email><![CDATA[ayse@example.com]]></wp:author_email><wp:author_display_name><![CDATA[Ayşe Yılmaz]]></wp:author_display_name></wp:author>
    <item>
        <title><![CDATA[Lemon & honey cake]]></title>
        <link>https://ayseskitchen.com/2024/05/lemon-cake/</link>
        <dc:creator><![CDATA[ayse]]></dc:creator>
        <content:encoded><![CDATA[<!-- wp:paragraph -->
<p>Bake it.</p>
<!-- /wp:paragraph -->]]></content:encoded>
        <wp:post_id>12</wp:post_id>
        <wp:post_date_gmt><![CDATA[2024-05-01 09:30:00]]></wp:post_date_gmt>
        <wp:post_name><![CDATA[lemon-cake]]></wp:post_name>
        <wp:status><![CDATA[publish]]></wp:status>
        <wp:post_type><![CDATA[post]]></wp:post_type>
        <category domain="category" nicename="desserts"><![CDATA[Desserts]]></category>
        <category domain="post_tag" nicename="a-b">A &amp; B</category>
        <category domain="post_format" nicename="post-format-aside"><![CDATA[Aside]]></category>
        <wp:postmeta><wp:meta_key><![CDATA[_thumbnail_id]]></wp:meta_key><wp:meta_value><![CDATA[30]]></wp:meta_value></wp:postmeta>
        <wp:comment>
            <wp:comment_author><![CDATA[Reader]]></wp:comment_author>
            <wp:comment_date_gmt><![CDATA[2024-05-02 10:00:00]]></wp:comment_date_gmt>
            <wp:comment_content><![CDATA[Lovely!]]></wp:comment_content>
            <wp:comment_approved><![CDATA[1]]></wp:comment_approved>
            <wp:comment_type><![CDATA[comment]]></wp:comment_type>
        </wp:comment>
    </item>
    <item>
        <title>cake.jpg</title>
        <wp:post_id>30</wp:post_id>
        <wp:post_type><![CDATA[attachment]]></wp:post_type>
        <wp:attachment_url><![CDATA[https://ayseskitchen.com/wp-content/uploads/2024/05/cake.jpg]]></wp:attachment_url>
    </item>
</channel>
</rss>"#;

    #[test]
    fn reads_the_site_people_posts_and_comments() {
        let export = read(SAMPLE.as_bytes()).unwrap();
        assert_eq!(export.site_title, "Ayşe's Kitchen");
        assert_eq!(export.site_url, "https://ayseskitchen.com");
        assert_eq!(export.authors.len(), 1);
        assert_eq!(export.authors[0].email, "ayse@example.com");
        assert_eq!(export.authors[0].name, "Ayşe Yılmaz");

        let post = &export.items[0];
        assert_eq!(post.title, "Lemon & honey cake");
        assert_eq!(post.kind, "post");
        assert_eq!(post.status, "publish");
        assert_eq!(post.slug, "lemon-cake");
        assert_eq!(post.date_utc, "2024-05-01 09:30:00");
        assert_eq!(post.author_login, "ayse");
        // Categories and tags; post formats aren't tags.
        assert_eq!(post.tags, ["Desserts", "A & B"]);
        assert_eq!(post.cover_id, Some(30));
        assert_eq!(post.comments.len(), 1);
        assert!(post.comments[0].approved);
        assert_eq!(post.comments[0].content, "Lovely!");
        assert_eq!(without_block_markers(&post.content), "<p>Bake it.</p>");

        let image = &export.items[1];
        assert_eq!(image.kind, "attachment");
        assert!(image.attachment_url.ends_with("/2024/05/cake.jpg"));
    }

    #[test]
    fn other_files_are_refused_with_a_clear_message() {
        assert_eq!(
            read(b"<html><body>Hi</body></html>").unwrap_err(),
            NOT_WORDPRESS
        );
        assert_eq!(read(b"not xml at all <<<").unwrap_err(), NOT_WORDPRESS);
        assert_eq!(read(b"").unwrap_err(), NOT_WORDPRESS);
        // A real export, cut off in the middle.
        let cut = &SAMPLE[..SAMPLE.len() / 2];
        assert_eq!(read(cut.as_bytes()).unwrap_err(), DAMAGED);
    }
}
