use crate::models::{AppState, Media};
use crate::templates::{MediaPickerTemplate, MediaTemplate, MediaView};
use crate::utils::{get_setting, or_log};

use axum::extract::{Multipart, Path, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Redirect, Response};
use image::codecs::jpeg::JpegEncoder;
use image::{DynamicImage, ImageDecoder, ImageFormat, ImageReader, Limits};
use rand::RngCore;
use std::io::Cursor;
use std::path::PathBuf;

pub const UPLOAD_DIR: &str = "uploads";
/// Maximum accepted upload size per request.
pub const MAX_UPLOAD_BYTES: usize = 20 * 1024 * 1024;
/// Larger photos are scaled down to fit this box.
const MAX_DIMENSION: u32 = 2000;
const JPEG_QUALITY: u8 = 85;

/// A validated, processed upload ready to be written to disk.
struct ProcessedImage {
    bytes: Vec<u8>,
    extension: &'static str,
    mime_type: &'static str,
    width: u32,
    height: u32,
}

async fn all_media(state: &AppState) -> Vec<MediaView> {
    or_log(
        sqlx::query_as::<_, Media>(
            "SELECT id, filename, original_name, size_bytes, width, height
             FROM media ORDER BY id DESC",
        )
        .fetch_all(&state.pool)
        .await,
        "list media",
    )
    .into_iter()
    .map(MediaView::from)
    .collect()
}

/// GET /admin/media -> Media library.
pub async fn media_page(State(state): State<AppState>) -> impl IntoResponse {
    MediaTemplate {
        blog_name: get_setting(&state.pool, "blog_name", "Bloogla").await,
        media: all_media(&state).await,
        error: None,
    }
}

/// POST /admin/media -> Upload one or more images.
pub async fn upload_media(State(state): State<AppState>, multipart: Multipart) -> Response {
    let errors = receive_uploads(&state, multipart).await;
    if errors.is_empty() {
        return Redirect::to("/admin/media").into_response();
    }

    (
        StatusCode::UNPROCESSABLE_ENTITY,
        MediaTemplate {
            blog_name: get_setting(&state.pool, "blog_name", "Bloogla").await,
            media: all_media(&state).await,
            error: Some(errors.join(" · ")),
        },
    )
        .into_response()
}

/// GET /admin/media/picker -> Media chooser for the post editor.
pub async fn picker(State(state): State<AppState>) -> impl IntoResponse {
    MediaPickerTemplate {
        media: all_media(&state).await,
        error: None,
    }
}

/// POST /admin/media/picker -> Upload from the chooser, then show it again.
pub async fn picker_upload(
    State(state): State<AppState>,
    multipart: Multipart,
) -> impl IntoResponse {
    let errors = receive_uploads(&state, multipart).await;
    MediaPickerTemplate {
        media: all_media(&state).await,
        error: (!errors.is_empty()).then(|| errors.join(" · ")),
    }
}

/// Process and save every file in the `files` field; returns one message per failure.
async fn receive_uploads(state: &AppState, mut multipart: Multipart) -> Vec<String> {
    let mut errors = Vec::new();

    loop {
        let field = match multipart.next_field().await {
            Ok(Some(field)) => field,
            Ok(None) => break,
            Err(e) => {
                errors.push(format!("Upload failed: {e}"));
                break;
            }
        };
        if field.name() != Some("files") {
            continue;
        }

        let original_name = field.file_name().unwrap_or("upload").to_string();
        let data = match field.bytes().await {
            Ok(data) if data.is_empty() => continue,
            Ok(data) => data,
            Err(e) => {
                errors.push(format!("{original_name}: {e}"));
                continue;
            }
        };

        let processed = tokio::task::spawn_blocking(move || process_image(&data)).await;
        let image = match processed {
            Ok(Ok(image)) => image,
            Ok(Err(e)) => {
                errors.push(format!("{original_name}: {e}"));
                continue;
            }
            Err(e) => {
                errors.push(format!("{original_name}: processing crashed ({e})"));
                continue;
            }
        };

        if let Err(e) = save_image(state, &original_name, image).await {
            errors.push(format!("{original_name}: {e}"));
        }
    }

    errors
}

/// DELETE /admin/media/:id -> Remove a file from the library and disk.
pub async fn delete_media(State(state): State<AppState>, Path(id): Path<i64>) -> StatusCode {
    let filename: Option<String> = or_log(
        sqlx::query_scalar("DELETE FROM media WHERE id = ? RETURNING filename")
            .bind(id)
            .fetch_optional(&state.pool)
            .await,
        "delete media",
    );

    match filename {
        Some(filename) => {
            if let Err(e) = tokio::fs::remove_file(PathBuf::from(UPLOAD_DIR).join(&filename)).await
            {
                eprintln!("Failed to remove upload {filename}: {e}");
            }
            StatusCode::OK
        }
        None => StatusCode::NOT_FOUND,
    }
}

/// Validate an upload by its content (not its name) and normalize it.
///
/// JPEG and PNG are decoded, rotated upright, scaled down to [`MAX_DIMENSION`]
/// and re-encoded, which also strips metadata such as GPS location. GIF and
/// WebP are kept byte-for-byte so animations survive.
fn process_image(data: &[u8]) -> Result<ProcessedImage, String> {
    let format = image::guess_format(data)
        .map_err(|_| "not a supported image (JPEG, PNG, GIF or WebP)".to_string())?;

    let mut limits = Limits::default();
    limits.max_image_width = Some(12_000);
    limits.max_image_height = Some(12_000);

    let mut reader = ImageReader::with_format(Cursor::new(data), format);
    reader.limits(limits);

    match format {
        ImageFormat::Gif | ImageFormat::WebP => {
            let (width, height) = reader
                .into_dimensions()
                .map_err(|e| format!("could not read image: {e}"))?;
            let (extension, mime_type) = if format == ImageFormat::Gif {
                ("gif", "image/gif")
            } else {
                ("webp", "image/webp")
            };
            Ok(ProcessedImage {
                bytes: data.to_vec(),
                extension,
                mime_type,
                width,
                height,
            })
        }
        ImageFormat::Jpeg | ImageFormat::Png => {
            let mut decoder = reader
                .into_decoder()
                .map_err(|e| format!("could not read image: {e}"))?;
            let orientation = decoder
                .orientation()
                .map_err(|e| format!("could not read image: {e}"))?;
            let mut img = DynamicImage::from_decoder(decoder)
                .map_err(|e| format!("could not decode image: {e}"))?;
            img.apply_orientation(orientation);

            if img.width() > MAX_DIMENSION || img.height() > MAX_DIMENSION {
                img = img.resize(
                    MAX_DIMENSION,
                    MAX_DIMENSION,
                    image::imageops::FilterType::Lanczos3,
                );
            }

            let mut bytes = Vec::new();
            let (extension, mime_type) = if format == ImageFormat::Jpeg {
                let encoder = JpegEncoder::new_with_quality(&mut bytes, JPEG_QUALITY);
                img.to_rgb8()
                    .write_with_encoder(encoder)
                    .map_err(|e| format!("could not encode image: {e}"))?;
                ("jpg", "image/jpeg")
            } else {
                img.write_to(&mut Cursor::new(&mut bytes), ImageFormat::Png)
                    .map_err(|e| format!("could not encode image: {e}"))?;
                ("png", "image/png")
            };

            Ok(ProcessedImage {
                bytes,
                extension,
                mime_type,
                width: img.width(),
                height: img.height(),
            })
        }
        _ => Err("not a supported image (JPEG, PNG, GIF or WebP)".to_string()),
    }
}

/// Write a processed image under a random name and record it in the library.
async fn save_image(
    state: &AppState,
    original_name: &str,
    image: ProcessedImage,
) -> Result<(), String> {
    let mut id = [0u8; 12];
    rand::thread_rng().fill_bytes(&mut id);
    let hex: String = id.iter().map(|b| format!("{b:02x}")).collect();
    let filename = format!("{hex}.{}", image.extension);

    tokio::fs::create_dir_all(UPLOAD_DIR)
        .await
        .map_err(|e| format!("could not create upload folder: {e}"))?;
    tokio::fs::write(PathBuf::from(UPLOAD_DIR).join(&filename), &image.bytes)
        .await
        .map_err(|e| format!("could not save file: {e}"))?;

    let original_name: String = original_name.chars().take(200).collect();
    sqlx::query(
        "INSERT INTO media (filename, original_name, mime_type, size_bytes, width, height)
         VALUES (?, ?, ?, ?, ?, ?)",
    )
    .bind(&filename)
    .bind(&original_name)
    .bind(image.mime_type)
    .bind(image.bytes.len() as i64)
    .bind(image.width as i64)
    .bind(image.height as i64)
    .execute(&state.pool)
    .await
    .map_err(|e| {
        eprintln!("Failed to record upload: {e}");
        "could not save to the media library".to_string()
    })?;

    Ok(())
}
