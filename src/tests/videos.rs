//! Short videos in Media: uploading them, where they were filmed removed,
//! choosing them in the editor, and visitors watching them.

use super::TestSite;
use crate::services::videos::tests::{mp4_box, phone_mp4};
use axum::body::Body;
use axum::http::{header, Request, StatusCode};

#[tokio::test]
async fn a_phone_video_is_kept_without_where_it_was_filmed() {
    let mut site = TestSite::with_owner().await;
    let page = site
        .upload("/admin/media", "files", "beach day.mp4", &phone_mp4())
        .await;
    assert_eq!(page.status, StatusCode::SEE_OTHER, "{}", page.body);
    let (filename, mime, name): (String, String, String) =
        sqlx::query_as("SELECT filename, mime_type, original_name FROM media")
            .fetch_one(&site.pool)
            .await
            .unwrap();
    assert!(filename.ends_with(".mp4"));
    assert_eq!(
        (mime.as_str(), name.as_str()),
        ("video/mp4", "beach day.mp4")
    );

    let uploads = std::path::Path::new(crate::services::images::UPLOAD_DIR);
    let saved = std::fs::read(uploads.join(&filename)).unwrap();
    assert_eq!(saved.len(), phone_mp4().len());
    assert!(
        !saved.windows(8).any(|w| w == b"+41.0128"),
        "the location is still readable"
    );
    assert!(!saved.windows(4).any(|w| w == b"udta"));
    // No half-finished upload is left behind.
    let leftovers = std::fs::read_dir(uploads)
        .unwrap()
        .flatten()
        .filter(|f| f.file_name().to_string_lossy().starts_with(".upload-"))
        .count();
    assert_eq!(leftovers, 0);

    // In the library as a video, without a description field.
    let library = site.get("/admin/media").await.body;
    assert!(library.contains(&format!(r#"<video src="/uploads/{filename}#t=0.1""#)));
    assert!(library.contains("Delete this video?"));

    // Visitors can watch it, and jump ahead (browsers ask for parts).
    site.new_visitor();
    let address = format!("/uploads/{filename}");
    let whole = site.get(&address).await;
    assert_eq!(whole.status, StatusCode::OK);
    assert_eq!(whole.header("content-type"), "video/mp4");
    let part = site
        .send(
            Request::get(&address).header(header::RANGE, "bytes=0-15"),
            Body::empty(),
        )
        .await;
    assert_eq!(part.status, StatusCode::PARTIAL_CONTENT);
    assert_eq!(part.bytes.len(), 16);
}

#[tokio::test]
async fn webm_videos_go_in_and_other_films_are_refused_in_plain_words() {
    let mut site = TestSite::with_owner().await;
    let webm = [
        &[0x1A, 0x45, 0xDF, 0xA3, 0x9F, 0x42, 0x82, 0x84][..],
        b"webm",
        &[0u8; 200],
    ]
    .concat();
    let page = site
        .upload("/admin/media", "files", "clip.webm", &webm)
        .await;
    assert_eq!(page.status, StatusCode::SEE_OTHER);

    let quicktime = [
        mp4_box(b"ftyp", b"qt  \0\0\0\0"),
        mp4_box(b"mdat", &[1; 40]),
    ]
    .concat();
    let refused = site
        .upload("/admin/media", "files", "film.mov", &quicktime)
        .await;
    assert_eq!(refused.status, StatusCode::UNPROCESSABLE_ENTITY);
    assert!(refused
        .body
        .contains("film.mov: not a supported video: save it as MP4 or WebM"));

    let broken = phone_mp4()[..80].to_vec();
    let refused = site
        .upload("/admin/media", "files", "cut.mp4", &broken)
        .await;
    assert!(refused.body.contains("cut.mp4: could not read video"));

    let count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM media")
        .fetch_one(&site.pool)
        .await
        .unwrap();
    assert_eq!(count, 1);
}

#[tokio::test]
async fn the_editor_chooses_videos_and_pictures_separately_and_posts_play_them() {
    let mut site = TestSite::with_owner().await;
    site.upload("/admin/media", "files", "clip.mp4", &phone_mp4())
        .await;
    let filename: String = sqlx::query_scalar("SELECT filename FROM media")
        .fetch_one(&site.pool)
        .await
        .unwrap();

    let videos = site.get("/admin/media/picker?videos=1").await.body;
    assert!(videos.contains(&format!(r#"data-url="/uploads/{filename}""#)));
    assert!(videos.contains("Or a video on YouTube or Vimeo"));
    let pictures = site.get("/admin/media/picker").await.body;
    assert!(!pictures.contains(&filename));

    let content = format!("Look:\n\n[video /uploads/{filename}]");
    site.post(
        "/admin/posts",
        &[
            ("title", "Waves"),
            ("content", &content),
            ("status", "published"),
        ],
    )
    .await;
    site.new_visitor();
    let post = site.get("/post/waves").await.body;
    assert!(post.contains(&format!(
        r#"<video class="media" controls preload="metadata" src="/uploads/{filename}"></video>"#
    )));
}

#[tokio::test]
async fn a_video_over_100_mb_is_refused_and_leaves_nothing_behind() {
    let mut site = TestSite::with_owner().await;
    let mut huge = phone_mp4();
    huge.resize(101 * 1024 * 1024, 0);
    let refused = site
        .upload("/admin/media", "files", "long.mp4", &huge)
        .await;
    assert!(refused.body.contains(
        "long.mp4: a video larger than 100 MB: make it shorter, or post it on YouTube or Vimeo"
    ));
    let leftovers = std::fs::read_dir(crate::services::images::UPLOAD_DIR)
        .map(|dir| {
            dir.flatten()
                .filter(|f| f.file_name().to_string_lossy().starts_with(".upload-"))
                .count()
        })
        .unwrap_or(0);
    assert_eq!(leftovers, 0);
    let count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM media")
        .fetch_one(&site.pool)
        .await
        .unwrap();
    assert_eq!(count, 0);
}
