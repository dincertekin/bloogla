//! Uploading pictures: the smaller WebP copy browsers get instead.

use super::TestSite;
use axum::body::Body;
use axum::http::{header, Request, StatusCode};
use image::ImageEncoder;

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

/// Send several files in one upload, the way the Media screen does.
async fn upload_several(site: &mut TestSite, files: &[(&str, &[u8])]) -> super::Page {
    let boundary = "bloogla-test-boundary";
    let mut body = Vec::new();
    for (name, bytes) in files {
        body.extend_from_slice(
            format!(
                "--{boundary}\r\nContent-Disposition: form-data; name=\"files\"; filename=\"{name}\"\r\n\
                 Content-Type: application/octet-stream\r\n\r\n"
            )
            .as_bytes(),
        );
        body.extend_from_slice(bytes);
        body.extend_from_slice(b"\r\n");
    }
    body.extend_from_slice(format!("--{boundary}--\r\n").as_bytes());
    let request = Request::post("/admin/media")
        .header(
            header::CONTENT_TYPE,
            format!("multipart/form-data; boundary={boundary}"),
        )
        .header("sec-fetch-site", "same-origin");
    site.send(request, Body::from(body)).await
}

async fn media_count(site: &TestSite) -> i64 {
    sqlx::query_scalar("SELECT COUNT(*) FROM media")
        .fetch_one(&site.pool)
        .await
        .unwrap()
}

/// Noise doesn't compress, so the file comes out about as large as its pixels.
fn noise(width: u32, height: u32, seed: u32) -> image::RgbaImage {
    image::RgbaImage::from_fn(width, height, |x, y| {
        let n = (x.wrapping_mul(2654435761) ^ y.wrapping_mul(40503) ^ seed).wrapping_mul(97);
        image::Rgba([n as u8, (n >> 8) as u8, (n >> 16) as u8, 255])
    })
}

/// An animated GIF of `frames` frames of noise.
fn animated_gif(width: u32, height: u32, frames: u32) -> Vec<u8> {
    let mut bytes = Vec::new();
    let mut encoder = image::codecs::gif::GifEncoder::new(&mut bytes);
    for seed in 0..frames {
        encoder
            .encode_frame(image::Frame::new(noise(width, height, seed)))
            .unwrap();
    }
    drop(encoder);
    bytes
}

#[tokio::test]
async fn a_file_that_is_too_large_doesnt_stop_the_others() {
    let mut site = TestSite::with_owner().await;
    let too_large = vec![0u8; 21 * 1024 * 1024];
    let page = upload_several(
        &mut site,
        &[
            ("huge.jpg", &too_large),
            ("beach.jpg", &photo()),
            ("lake.jpg", &photo()),
        ],
    )
    .await;
    assert!(
        page.body
            .contains("huge.jpg: larger than 20 MB: save it smaller and upload it again"),
        "{}",
        page.body
    );
    assert_eq!(media_count(&site).await, 2, "the other two are saved");
}

#[tokio::test]
async fn a_large_webp_is_made_smaller_with_copies_for_phones() {
    let mut site = TestSite::with_owner().await;
    let photo = image::load_from_memory(&photo()).unwrap();
    let big = photo.resize_exact(3000, 2000, image::imageops::FilterType::Triangle);
    let webp = webp::Encoder::from_rgb(big.to_rgb8().as_raw(), 3000, 2000)
        .encode(80.0)
        .to_vec();
    site.upload("/admin/media", "files", "phone.webp", &webp)
        .await;

    let (filename, width, small, thumb): (String, i64, Option<String>, Option<String>) =
        sqlx::query_as("SELECT filename, width, small_filename, thumb_filename FROM media")
            .fetch_one(&site.pool)
            .await
            .unwrap();
    assert!(filename.ends_with(".webp"));
    assert_eq!(width, 2000);
    let uploads = std::path::Path::new(crate::handlers::admin::media::UPLOAD_DIR);
    assert_eq!(
        image::image_dimensions(uploads.join(&filename)).unwrap(),
        (2000, 1333)
    );
    let small = small.expect("an 800px copy");
    assert_eq!(
        image::image_dimensions(uploads.join(&small)).unwrap().0,
        800
    );
    let thumb = thumb.expect("a 400px copy");
    assert_eq!(
        image::image_dimensions(uploads.join(&thumb)).unwrap().0,
        400
    );
}

#[tokio::test]
async fn small_animations_are_kept_and_large_ones_refused() {
    let mut site = TestSite::with_owner().await;
    let small = animated_gif(100, 100, 3);
    site.upload("/admin/media", "files", "wave.gif", &small)
        .await;
    let saved: Vec<u8> = {
        let filename: String = sqlx::query_scalar("SELECT filename FROM media")
            .fetch_one(&site.pool)
            .await
            .unwrap();
        let uploads = std::path::Path::new(crate::handlers::admin::media::UPLOAD_DIR);
        std::fs::read(uploads.join(filename)).unwrap()
    };
    assert_eq!(saved, small, "the animation is kept as it is");

    // Making a real 5 MB animation is slow in tests; the same small one with
    // 5 MB of filler after its end counts as just as large.
    let mut large = small.clone();
    large.resize(small.len() + 5 * 1024 * 1024, 0);
    let page = site
        .upload("/admin/media", "files", "film.gif", &large)
        .await;
    assert!(
        page.body
            .contains("film.gif: an animation larger than 5 MB"),
        "{}",
        page.body
    );
    assert_eq!(media_count(&site).await, 1);
}

#[tokio::test]
async fn a_picture_too_large_to_open_gets_a_plain_message() {
    let mut site = TestSite::with_owner().await;
    // A PNG whose header claims 20,000 × 20,000 pixels.
    let mut png = Vec::new();
    image::codecs::png::PngEncoder::new(&mut png)
        .write_image(&[0; 4], 1, 1, image::ExtendedColorType::Rgba8)
        .unwrap();
    png[16..20].copy_from_slice(&20_000u32.to_be_bytes());
    png[20..24].copy_from_slice(&20_000u32.to_be_bytes());
    // The header's checksum, so it reads as a real (if huge) picture.
    let checksum = crc32(&png[12..29]);
    png[29..33].copy_from_slice(&checksum.to_be_bytes());
    let page = site
        .upload("/admin/media", "files", "poster.png", &png)
        .await;
    assert!(
        page.body
            .contains("poster.png: more than 12,000 pixels wide or tall"),
        "{}",
        page.body
    );
}

/// The checksum PNG uses for each of its parts (CRC-32).
fn crc32(bytes: &[u8]) -> u32 {
    let mut crc = 0xFFFF_FFFFu32;
    for &byte in bytes {
        crc ^= byte as u32;
        for _ in 0..8 {
            crc = if crc & 1 == 1 {
                (crc >> 1) ^ 0xEDB8_8320
            } else {
                crc >> 1
            };
        }
    }
    !crc
}
