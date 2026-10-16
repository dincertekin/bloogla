//! Settings → Import & export: importing a WordPress site, and exporting.

use super::{TestSite, ADMIN_EMAIL};
use axum::http::StatusCode;

/// An export from "Ayşe's Kitchen". Its address is this machine, so the
/// images in it must be refused, never downloaded.
fn export_file() -> String {
    format!(
        r#"<?xml version="1.0" encoding="UTF-8" ?>
<rss version="2.0" xmlns:content="http://purl.org/rss/1.0/modules/content/"
    xmlns:dc="http://purl.org/dc/elements/1.1/" xmlns:wp="http://wordpress.org/export/1.2/">
<channel>
    <title>Ayşe's Kitchen</title>
    <link>http://127.0.0.1:9</link>
    <wp:wxr_version>1.2</wp:wxr_version>
    <wp:base_blog_url>http://127.0.0.1:9</wp:base_blog_url>
    <wp:author><wp:author_login>owner</wp:author_login><wp:author_email>{ADMIN_EMAIL}</wp:author_email><wp:author_display_name>Owner</wp:author_display_name></wp:author>
    <wp:author><wp:author_login>mehmet</wp:author_login><wp:author_email>mehmet@example.com</wp:author_email><wp:author_display_name>Mehmet Kaya</wp:author_display_name></wp:author>
    <item>
        <title>Lemon cake</title>
        <link>http://127.0.0.1:9/2024/05/lemon-cake/</link>
        <dc:creator>owner</dc:creator>
        <content:encoded><![CDATA[<!-- wp:paragraph --><p>Bake it.</p><!-- /wp:paragraph -->
<img src="http://127.0.0.1:9/wp-content/uploads/2024/05/cake-300x200.jpg" srcset="http://127.0.0.1:9/wp-content/uploads/2024/05/cake-300x200.jpg 300w" alt="Cake">]]></content:encoded>
        <wp:post_id>12</wp:post_id>
        <wp:post_date_gmt>2024-05-01 09:30:00</wp:post_date_gmt>
        <wp:post_name>lemon-cake</wp:post_name>
        <wp:status>publish</wp:status>
        <wp:post_type>post</wp:post_type>
        <category domain="category" nicename="desserts"><![CDATA[Desserts]]></category>
        <wp:comment><wp:comment_author>Reader</wp:comment_author><wp:comment_date_gmt>2024-05-02 10:00:00</wp:comment_date_gmt><wp:comment_content>Lovely!</wp:comment_content><wp:comment_approved>1</wp:comment_approved><wp:comment_type>comment</wp:comment_type></wp:comment>
        <wp:comment><wp:comment_author>Spammer</wp:comment_author><wp:comment_content>Buy now</wp:comment_content><wp:comment_approved>spam</wp:comment_approved></wp:comment>
        <wp:comment><wp:comment_author>Other blog</wp:comment_author><wp:comment_content>Linked</wp:comment_content><wp:comment_approved>1</wp:comment_approved><wp:comment_type>pingback</wp:comment_type></wp:comment>
    </item>
    <item>
        <title>Künefe</title>
        <link>http://127.0.0.1:9/k%c3%bcnefe/</link>
        <dc:creator>mehmet</dc:creator>
        <content:encoded><![CDATA[Not finished.]]></content:encoded>
        <wp:post_id>13</wp:post_id>
        <wp:post_date_gmt>0000-00-00 00:00:00</wp:post_date_gmt>
        <wp:post_date>2024-06-01 12:00:00</wp:post_date>
        <wp:post_name>k%c3%bcnefe</wp:post_name>
        <wp:status>draft</wp:status>
        <wp:post_type>post</wp:post_type>
    </item>
    <item>
        <title>Our story</title>
        <link>http://127.0.0.1:9/our-story/</link>
        <dc:creator>owner</dc:creator>
        <content:encoded><![CDATA[<p>Since 2010.</p>]]></content:encoded>
        <wp:post_id>14</wp:post_id>
        <wp:post_date_gmt>2020-01-01 08:00:00</wp:post_date_gmt>
        <wp:post_name>our-story</wp:post_name>
        <wp:status>publish</wp:status>
        <wp:post_type>page</wp:post_type>
    </item>
    <item>
        <title>Hello world</title>
        <link>http://127.0.0.1:9/hello-world/</link>
        <dc:creator>owner</dc:creator>
        <content:encoded><![CDATA[WordPress's first post.]]></content:encoded>
        <wp:post_id>15</wp:post_id>
        <wp:post_name>hello-world</wp:post_name>
        <wp:status>publish</wp:status>
        <wp:post_type>post</wp:post_type>
    </item>
    <item>
        <title>Thrown away</title>
        <wp:post_id>16</wp:post_id>
        <wp:post_name>thrown-away</wp:post_name>
        <wp:status>trash</wp:status>
        <wp:post_type>post</wp:post_type>
    </item>
</channel>
</rss>"#
    )
}

/// Start from the upload form (the import is shared by the whole server, and
/// tests run one after another).
fn fresh_start() {
    crate::services::import::reset();
}

/// Ask for the Import card until the import has finished.
async fn wait_for_report(site: &mut TestSite) -> String {
    for _ in 0..200 {
        let card = site.get("/admin/import").await.body;
        if card.contains("Import another file") {
            return card;
        }
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
    }
    panic!("the import didn't finish");
}

async fn count(site: &TestSite, sql: &str) -> i64 {
    sqlx::query_scalar(sql).fetch_one(&site.pool).await.unwrap()
}

#[tokio::test]
async fn other_files_are_refused_with_what_to_do() {
    let mut site = TestSite::with_owner().await;
    fresh_start();
    let settings = site.get("/admin/settings").await.body;
    assert!(settings.contains(r#"<label for="import-file">WordPress export file (.xml)</label>"#));

    let card = site
        .upload(
            "/admin/import/check",
            "file",
            "photo.html",
            b"<html><body>Hi</body></html>",
        )
        .await;
    assert_eq!(card.status, StatusCode::OK);
    assert!(card
        .body
        .contains("The file is not a WordPress export. In WordPress, use Tools → Export."));
}

#[tokio::test]
async fn a_wordpress_site_comes_in_with_its_addresses() {
    let mut site = TestSite::with_owner().await;
    fresh_start();
    // A post here already uses the address of WordPress's "Hello world".
    site.publish("Hello world", "published").await;

    // 1. Check: what's in the file, before anything changes.
    let checked = site
        .upload(
            "/admin/import/check",
            "file",
            "kitchen.xml",
            export_file().as_bytes(),
        )
        .await
        .body;
    assert!(checked.contains("This file is from Ayşe"), "{checked}");
    assert!(checked.contains("2 posts"), "{checked}");
    assert!(checked.contains("1 page"));
    assert!(checked.contains("1 comment"));
    assert!(checked.contains("1 image"));
    // The owner is found by email; Mehmet isn't on this site.
    assert!(checked.contains("Found"));
    assert!(checked.contains("Their posts will be yours."));
    // (The page writes the apostrophe in "they're" as an HTML code.)
    assert!(checked.contains("skipped: Hello world"));
    assert_eq!(count(&site, "SELECT COUNT(*) FROM posts").await, 1);

    // 2. Import, with the images and comments.
    site.post(
        "/admin/import/start",
        &[("copy_images", "1"), ("bring_comments", "1")],
    )
    .await;
    let report = wait_for_report(&mut site).await;
    assert!(report.contains("Imported from Ayşe"), "{report}");
    // The image is on this machine, so it was refused, not downloaded.
    assert!(report.contains("private network"), "{report}");
    assert!(report.contains("127.0.0.1:9/wp-content/uploads/2024/05/cake.jpg"));
    assert!(report.contains("skipped because their address is already used: Hello world"));

    // Posts, pages, drafts and the trash.
    assert_eq!(count(&site, "SELECT COUNT(*) FROM posts").await, 4);
    assert_eq!(
        count(
            &site,
            "SELECT COUNT(*) FROM posts WHERE slug = 'kunefe' AND status = 'draft'"
        )
        .await,
        1
    );
    assert_eq!(
        count(
            &site,
            "SELECT COUNT(*) FROM posts WHERE slug = 'our-story' AND is_page = 1"
        )
        .await,
        1
    );
    assert_eq!(
        count(
            &site,
            "SELECT COUNT(*) FROM posts WHERE slug = 'thrown-away'"
        )
        .await,
        0
    );
    let (content, published_at): (String, String) =
        sqlx::query_as("SELECT content, published_at FROM posts WHERE slug = 'lemon-cake'")
            .fetch_one(&site.pool)
            .await
            .unwrap();
    assert_eq!(published_at, "2024-05-01 09:30:00");
    assert!(!content.contains("<!-- wp:"));
    // Everything went to the owner: found by email, or the one importing.
    assert_eq!(
        count(&site, "SELECT COUNT(DISTINCT author_id) FROM posts").await,
        1
    );
    // The category is a tag; only the approved comment came along.
    assert_eq!(
        count(&site, "SELECT COUNT(*) FROM tags WHERE name = 'Desserts'").await,
        1
    );
    assert_eq!(count(&site, "SELECT COUNT(*) FROM comments").await, 1);

    // A backup was saved first.
    let backups = std::fs::read_dir(site.folder.join("data/backups")).unwrap();
    assert!(backups.flatten().any(|f| f
        .file_name()
        .to_string_lossy()
        .starts_with("before-import-")));

    // Old addresses lead to the new ones.
    let old = site.get("/2024/05/lemon-cake/").await;
    assert_eq!(old.status, StatusCode::PERMANENT_REDIRECT);
    assert_eq!(old.location(), "/post/lemon-cake");
    assert_eq!(site.get("/our-story/").await.location(), "/our-story");
    assert_eq!(
        site.get("/category/desserts/").await.location(),
        "/tag/desserts"
    );
    assert_eq!(
        site.get("/2024/05/nothing-here/").await.status,
        StatusCode::NOT_FOUND
    );

    // Back to the upload form.
    let card = site.post("/admin/import/reset", &[]).await.body;
    assert!(card.contains("Check file"));
}

#[tokio::test]
async fn a_second_click_on_import_brings_nothing_in_twice() {
    let mut site = TestSite::with_owner().await;
    fresh_start();
    site.upload(
        "/admin/import/check",
        "file",
        "kitchen.xml",
        export_file().as_bytes(),
    )
    .await;
    site.post("/admin/import/start", &[]).await;
    wait_for_report(&mut site).await;
    let posts = count(&site, "SELECT COUNT(*) FROM posts").await;

    // Sent again (a double click, or the back button): the report stays and
    // nothing comes in twice.
    let again = site.post("/admin/import/start", &[]).await.body;
    assert!(again.contains("Import another file"), "{again}");
    assert_eq!(count(&site, "SELECT COUNT(*) FROM posts").await, posts);
}

/// A site with a post (tag, comment, cover image), a draft and a page.
async fn site_to_export() -> TestSite {
    let mut site = TestSite::with_owner().await;
    site.publish("Spring menu", "published").await;
    site.publish("Half written", "draft").await;
    site.post(
        "/admin/pages",
        &[
            ("title", "About us"),
            ("content", "We cook."),
            ("status", "published"),
        ],
    )
    .await;
    // An image in Media (the file and its record), as the cover.
    std::fs::create_dir_all(site.folder.join("uploads")).unwrap();
    std::fs::write(site.folder.join("uploads/menu.jpg"), b"jpeg bytes").unwrap();
    for sql in [
        "INSERT INTO media (filename, original_name, mime_type, size_bytes) VALUES ('menu.jpg', 'menu.jpg', 'image/jpeg', 10)",
        "UPDATE posts SET cover_image = '/uploads/menu.jpg', content = 'See ![menu](/uploads/menu.jpg)' WHERE slug = 'spring-menu'",
        "INSERT INTO tags (name, slug) VALUES ('Seasonal', 'seasonal')",
        "INSERT INTO post_tags (post_id, tag_id) SELECT p.id, t.id FROM posts p, tags t WHERE p.slug = 'spring-menu'",
        "INSERT INTO comments (post_id, author_name, content, status) SELECT id, 'Reader', 'Yum', 'approved' FROM posts WHERE slug = 'spring-menu'",
        "INSERT INTO comments (post_id, author_name, content, status) SELECT id, 'Spammer', 'Buy', 'pending' FROM posts WHERE slug = 'spring-menu'",
    ] {
        sqlx::query(sql).execute(&site.pool).await.unwrap();
    }
    site
}

#[tokio::test]
async fn the_wordpress_file_brings_everything_back_in() {
    let mut site = site_to_export().await;
    let settings = site.get("/admin/settings").await.body;
    assert!(settings.contains(r#"href="/admin/export/wordpress.xml""#));

    let file = site.get("/admin/export/wordpress.xml").await;
    assert_eq!(file.status, StatusCode::OK);
    assert!(file
        .header("content-disposition")
        .starts_with("attachment;"));

    // Our own reader understands it, like WordPress would.
    let export = crate::services::import::wordpress::read(file.body.as_bytes()).unwrap();
    let find = |slug: &str| export.items.iter().find(|i| i.slug == slug).unwrap();
    let post = find("spring-menu");
    assert_eq!(post.status, "publish");
    assert_eq!(post.tags, ["Seasonal"]);
    assert_eq!(post.comments.len(), 1, "only approved comments");
    // Images have full addresses, so WordPress can fetch them.
    assert!(post.content.contains("/uploads/menu.jpg"));
    assert!(post.content.contains("://"));
    assert_eq!(find("half-written").status, "draft");
    assert_eq!(find("about-us").kind, "page");
    let image = export
        .items
        .iter()
        .find(|i| i.kind == "attachment")
        .unwrap();
    assert!(image.attachment_url.ends_with("/uploads/menu.jpg"));
    assert_eq!(post.cover_id, Some(image.wordpress_id));

    // And back in through the import, into the same site emptied. (A test
    // can't open a second site: test sites take turns.)
    for table in ["comments", "post_tags", "tags", "posts"] {
        sqlx::query(&format!("DELETE FROM {table}"))
            .execute(&site.pool)
            .await
            .unwrap();
    }
    fresh_start();
    site.upload(
        "/admin/import/check",
        "file",
        "export.xml",
        file.body.as_bytes(),
    )
    .await;
    site.post("/admin/import/start", &[("bring_comments", "1")])
        .await;
    wait_for_report(&mut site).await;
    assert_eq!(count(&site, "SELECT COUNT(*) FROM posts").await, 3);
    assert_eq!(
        count(&site, "SELECT COUNT(*) FROM posts WHERE status = 'draft'").await,
        1
    );
    assert_eq!(count(&site, "SELECT COUNT(*) FROM comments").await, 1);
    assert_eq!(
        count(&site, "SELECT COUNT(*) FROM tags WHERE name = 'Seasonal'").await,
        1
    );
}

#[tokio::test]
async fn markdown_files_come_with_their_images() {
    use std::io::Read;

    let mut site = site_to_export().await;
    let file = site.get("/admin/export/markdown.zip").await;
    assert_eq!(file.status, StatusCode::OK);
    assert_eq!(file.header("content-type"), "application/zip");

    let mut zip = zip::ZipArchive::new(std::io::Cursor::new(file.bytes)).unwrap();
    let mut read = |name: &str| {
        let mut text = String::new();
        zip.by_name(name)
            .unwrap()
            .read_to_string(&mut text)
            .unwrap();
        text
    };
    let post = read("posts/spring-menu.md");
    assert!(post.starts_with("---\ntitle: \"Spring menu\"\n"), "{post}");
    assert!(post.contains("tags: [\"Seasonal\"]"));
    assert!(post.contains("cover: \"/uploads/menu.jpg\""));
    assert!(post.ends_with("See ![menu](/uploads/menu.jpg)\n"));
    assert!(read("posts/half-written.md").contains("draft: true"));
    assert!(read("pages/about-us.md").contains("We cook."));
    assert_eq!(read("uploads/menu.jpg"), "jpeg bytes");
}
