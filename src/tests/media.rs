//! Uploading pictures: the smaller WebP copy browsers get instead.

use super::TestSite;
use axum::body::Body;
use axum::http::{header, Request, StatusCode};

/// A photo-like JPEG (soft colours, some texture), as a camera might make.
fn photo() -> Vec<u8> {
    let img = image::RgbImage::from_fn(1200, 800, |x, y| {
        let texture = ((x * 7 + y * 13) % 17) as u8;
        image::Rgb([
            (x / 5) as u8 ^ texture,
            (y / 4) as u8,
            (((x + y) / 8) as u8).wrapping_add(texture),
        ])
    });
    let mut bytes = Vec::new();
    img.write_with_encoder(image::codecs::jpeg::JpegEncoder::new_with_quality(
        &mut bytes, 90,
    ))
    .unwrap();
    bytes
}

/// Ask for an uploaded file the way a browser does.
async fn fetch(site: &mut TestSite, path: &str, accept: &str) -> super::Page {
    site.send(
        Request::get(path).header(header::ACCEPT, accept),
        Body::empty(),
    )
    .await
}

#[tokio::test]
async fn browsers_that_accept_webp_get_the_smaller_copy() {
    let mut site = TestSite::with_owner().await;
    let page = site
        .upload("/admin/media", "files", "beach.jpg", &photo())
        .await;
    assert!(page.status.is_success() || page.status.is_redirection());
    let (id, filename, small, thumb): (i64, String, Option<String>, Option<String>) =
        sqlx::query_as("SELECT id, filename, small_filename, thumb_filename FROM media")
            .fetch_one(&site.pool)
            .await
            .unwrap();
    let uploads = std::path::Path::new(crate::handlers::admin::media::UPLOAD_DIR);
    assert!(uploads.join(format!("{filename}.webp")).exists());
    let small = small.expect("an 800px copy");
    assert!(uploads.join(format!("{small}.webp")).exists());
    // And a 400px copy, for cards on phones.
    let thumb = thumb.expect("a 400px copy");
    let thumb_size = image::image_dimensions(uploads.join(&thumb)).unwrap();
    assert_eq!(thumb_size.0, 400);
    assert!(uploads.join(format!("{thumb}.webp")).exists());

    // Today's browsers: the WebP copy, smaller than the JPEG.
    let modern = "image/avif,image/webp,image/apng,image/*,*/*;q=0.8";
    let webp = fetch(&mut site, &format!("/uploads/{filename}"), modern).await;
    assert_eq!(webp.status, StatusCode::OK);
    assert_eq!(webp.header("content-type"), "image/webp");
    assert_eq!(webp.header("vary"), "Accept");
    // A browser that doesn't know WebP: the JPEG.
    let jpeg = fetch(&mut site, &format!("/uploads/{filename}"), "image/*").await;
    assert_eq!(jpeg.header("content-type"), "image/jpeg");
    assert!(webp.bytes.len() < jpeg.bytes.len(), "the copy is smaller");

    // Deleting the picture deletes its copies too.
    let deleted = site
        .send(
            Request::delete(format!("/admin/media/{id}")).header("sec-fetch-site", "same-origin"),
            Body::empty(),
        )
        .await;
    assert_eq!(deleted.status, StatusCode::OK);
    for name in [&filename, &small, &thumb] {
        assert!(!uploads.join(name).exists());
        assert!(!uploads.join(format!("{name}.webp")).exists());
    }
}

/// Images uploaded before 400px copies existed get theirs when Bloogla starts.
#[tokio::test]
async fn older_images_get_their_small_copy() {
    let mut site = TestSite::with_owner().await;
    site.upload("/admin/media", "files", "old.jpg", &photo())
        .await;
    // As Bloogla 1.2.1 left it: no 400px copy.
    let (id, thumb): (i64, String) = sqlx::query_as("SELECT id, thumb_filename FROM media")
        .fetch_one(&site.pool)
        .await
        .unwrap();
    let uploads = std::path::Path::new(crate::handlers::admin::media::UPLOAD_DIR);
    std::fs::remove_file(uploads.join(&thumb)).unwrap();
    let _ = std::fs::remove_file(uploads.join(format!("{thumb}.webp")));
    sqlx::query("UPDATE media SET thumb_filename = NULL")
        .execute(&site.pool)
        .await
        .unwrap();

    crate::handlers::admin::media::make_missing_thumbs(site.pool.clone()).await;
    let made: Option<String> = sqlx::query_scalar("SELECT thumb_filename FROM media WHERE id = ?")
        .bind(id)
        .fetch_one(&site.pool)
        .await
        .unwrap();
    let made = made.expect("the copy is recorded");
    assert_eq!(image::image_dimensions(uploads.join(&made)).unwrap().0, 400);

    // Clean up the files this test made in uploads/.
    let filename: String = sqlx::query_scalar("SELECT filename FROM media")
        .fetch_one(&site.pool)
        .await
        .unwrap();
    for name in [filename.clone(), made.clone()] {
        let _ = std::fs::remove_file(uploads.join(&name));
        let _ = std::fs::remove_file(uploads.join(format!("{name}.webp")));
    }
    let _ = std::fs::remove_file(uploads.join(filename.replace(".jpg", "-800.jpg")));
    let _ = std::fs::remove_file(uploads.join(filename.replace(".jpg", "-800.jpg.webp")));
}

/// A picture inside a post gets its smaller copies on the post page, so
/// phones don't download the full size; the feed keeps the plain address.
#[tokio::test]
async fn pictures_in_posts_offer_their_smaller_copies() {
    let mut site = TestSite::with_owner().await;
    site.upload("/admin/media", "files", "pie.jpg", &photo())
        .await;
    let (filename, small, thumb): (String, String, String) =
        sqlx::query_as("SELECT filename, small_filename, thumb_filename FROM media")
            .fetch_one(&site.pool)
            .await
            .unwrap();
    site.post(
        "/admin/posts",
        &[
            ("title", "Pie"),
            ("content", &format!("![The pie](/uploads/{filename})")),
            ("status", "published"),
        ],
    )
    .await;
    site.new_visitor();
    let page = site.get("/post/pie").await.body.replace("&#x2F;", "/");
    assert!(
        page.contains(&format!(
            r#"srcset="/uploads/{thumb} 400w, /uploads/{small} 800w, /uploads/{filename} 1200w""#
        )),
        "{page}"
    );
    assert!(page.contains(r#"width="1200" height="800""#));
    assert!(!site.get("/rss.xml").await.body.contains("srcset"));

    let uploads = std::path::Path::new(crate::handlers::admin::media::UPLOAD_DIR);
    for name in [&filename, &small, &thumb] {
        let _ = std::fs::remove_file(uploads.join(name));
        let _ = std::fs::remove_file(uploads.join(format!("{name}.webp")));
    }
}
