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
    let (id, filename, small): (i64, String, Option<String>) =
        sqlx::query_as("SELECT id, filename, small_filename FROM media")
            .fetch_one(&site.pool)
            .await
            .unwrap();
    let uploads = std::path::Path::new(crate::handlers::admin::media::UPLOAD_DIR);
    assert!(uploads.join(format!("{filename}.webp")).exists());
    let small = small.expect("an 800px copy");
    assert!(uploads.join(format!("{small}.webp")).exists());

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
    for name in [&filename, &small] {
        assert!(!uploads.join(name).exists());
        assert!(!uploads.join(format!("{name}.webp")).exists());
    }
}
