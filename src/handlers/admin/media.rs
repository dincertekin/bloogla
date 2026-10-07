//! The media library: uploading, listing and deleting images, and the image
//! chooser inside the post editor.
//!
//! Uploads are checked by their content (not their name), resized for the web
//! and stripped of metadata such as GPS location before they're saved.

use crate::app::models::{CurrentUser, Media};
use crate::app::state::AppState;
use crate::db::{or_log, settings};
use crate::i18n::Lang;

use askama::Template;
use axum::extract::{Extension, Multipart, Path, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Redirect, Response};
use image::codecs::jpeg::JpegEncoder;
use image::codecs::png::{CompressionType, PngEncoder};
use image::{DynamicImage, ImageDecoder, ImageFormat, ImageReader, Limits};
use std::io::Cursor;
use std::path::PathBuf;

/// Folder uploads are saved in, served at `/uploads`.
pub const UPLOAD_DIR: &str = "uploads";
/// Maximum accepted upload size per request.
pub const MAX_UPLOAD_BYTES: usize = 20 * 1024 * 1024;
/// Larger photos are scaled down to fit this box.
const MAX_DIMENSION: u32 = 2000;
const JPEG_QUALITY: u8 = 85;

/// Media library page template.
#[derive(Template)]
#[template(path = "media.html")]
pub struct MediaTemplate {
    pub blog_name: String,
    pub me: CurrentUser,
    pub media: Vec<MediaView>,
    pub error: Option<String>,
}

/// A media library entry prepared for display.
pub struct MediaView {
    pub id: i64,
    pub url: String,
    pub original_name: String,
    /// Alt text guessed from the file name.
    pub alt: String,
    pub markdown: String,
    pub size_label: String,
    pub dimensions: String,
}

impl From<Media> for MediaView {
    fn from(m: Media) -> Self {
        let url = format!("/uploads/{}", m.filename);
        let alt = m
            .original_name
            .rsplit_once('.')
            .map_or(m.original_name.as_str(), |(stem, _)| stem)
            .replace(['[', ']'], "");
        let size_label = if m.size_bytes >= 1024 * 1024 {
            format!("{:.1} MB", m.size_bytes as f64 / (1024.0 * 1024.0))
        } else {
            format!("{} KB", (m.size_bytes + 1023) / 1024)
        };
        let dimensions = match (m.width, m.height) {
            (Some(w), Some(h)) => format!("{w}×{h}"),
            _ => String::new(),
        };
        Self {
            id: m.id,
            markdown: format!("![{alt}]({url})"),
            alt,
            url,
            original_name: m.original_name,
            size_label,
            dimensions,
        }
    }
}

/// Media chooser shown inside the post editor.
#[derive(Template)]
#[template(path = "media_picker.html")]
pub struct MediaPickerTemplate {
    pub me: CurrentUser,
    pub media: Vec<MediaView>,
    pub error: Option<String>,
}

/// A validated, processed upload ready to be written to disk.
struct ProcessedImage {
    bytes: Vec<u8>,
    extension: &'static str,
    mime_type: &'static str,
    width: u32,
    height: u32,
    /// A copy [`SMALL_WIDTH`] pixels wide, for phones and post cards
    /// (only when the image is wider than that).
    small: Option<Vec<u8>>,
}

/// Width of the smaller copy made of wide photos.
const SMALL_WIDTH: u32 = 800;

/// Encode `img` in the upload's format (JPEG or PNG).
fn encode(img: &DynamicImage, format: ImageFormat) -> Result<Vec<u8>, String> {
    let mut bytes = Vec::new();
    let result = if format == ImageFormat::Jpeg {
        let encoder = JpegEncoder::new_with_quality(&mut bytes, JPEG_QUALITY);
        img.to_rgb8().write_with_encoder(encoder)
    } else {
        // Best compression: uploads are saved once and downloaded many times.
        let encoder = PngEncoder::new_with_quality(
            &mut bytes,
            CompressionType::Best,
            image::codecs::png::FilterType::Adaptive,
        );
        img.write_with_encoder(encoder)
    };
    result.map_err(|e| {
        tracing::warn!("Upload failed: {e}");
        "could not encode image".to_string()
    })?;
    Ok(bytes)
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
pub async fn page(
    State(state): State<AppState>,
    Extension(me): Extension<CurrentUser>,
) -> impl IntoResponse {
    MediaTemplate {
        blog_name: settings::load(&state.pool).await.blog_name.clone(),
        me,
        media: all_media(&state).await,
        error: None,
    }
}

/// POST /admin/media -> Upload one or more images.
pub async fn upload(
    State(state): State<AppState>,
    Extension(me): Extension<CurrentUser>,
    multipart: Multipart,
) -> Response {
    let errors = receive_uploads(&state, me.lang, multipart).await;
    if errors.is_empty() {
        return Redirect::to("/admin/media").into_response();
    }

    (
        StatusCode::UNPROCESSABLE_ENTITY,
        MediaTemplate {
            blog_name: settings::load(&state.pool).await.blog_name.clone(),
            me,
            media: all_media(&state).await,
            error: Some(errors.join(" · ")),
        },
    )
        .into_response()
}

/// GET /admin/media/picker -> Media chooser for the post editor.
pub async fn picker(
    State(state): State<AppState>,
    Extension(me): Extension<CurrentUser>,
) -> impl IntoResponse {
    MediaPickerTemplate {
        me,
        media: all_media(&state).await,
        error: None,
    }
}

/// POST /admin/media/picker -> Upload from the chooser, then show it again.
pub async fn picker_upload(
    State(state): State<AppState>,
    Extension(me): Extension<CurrentUser>,
    multipart: Multipart,
) -> impl IntoResponse {
    let errors = receive_uploads(&state, me.lang, multipart).await;
    MediaPickerTemplate {
        me,
        media: all_media(&state).await,
        error: (!errors.is_empty()).then(|| errors.join(" · ")),
    }
}

/// Process and save every file in the `files` field; returns one message per failure.
async fn receive_uploads(state: &AppState, lang: Lang, mut multipart: Multipart) -> Vec<String> {
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
                errors.push(format!("{original_name}: {}", lang.t_owned(&e)));
                continue;
            }
            Err(e) => {
                tracing::error!("Image processing crashed: {e}");
                errors.push(format!(
                    "{original_name}: {}",
                    lang.t("couldn't process this image")
                ));
                continue;
            }
        };

        if let Err(e) = save_image(state, &original_name, image).await {
            errors.push(format!("{original_name}: {}", lang.t_owned(&e)));
        }
    }

    errors
}

/// DELETE /admin/media/:id -> Remove a file from the library and disk.
pub async fn delete(State(state): State<AppState>, Path(id): Path<i64>) -> StatusCode {
    let files: Option<(String, Option<String>)> = or_log(
        sqlx::query_as("DELETE FROM media WHERE id = ? RETURNING filename, small_filename")
            .bind(id)
            .fetch_optional(&state.pool)
            .await,
        "delete media",
    );

    match files {
        Some((filename, small)) => {
            for name in std::iter::once(filename).chain(small) {
                if let Err(e) = tokio::fs::remove_file(PathBuf::from(UPLOAD_DIR).join(&name)).await
                {
                    tracing::error!("Failed to remove upload {name}: {e}");
                }
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
            let (width, height) = reader.into_dimensions().map_err(|e| {
                tracing::warn!("Upload failed: {e}");
                "could not read image".to_string()
            })?;
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
                small: None,
            })
        }
        ImageFormat::Jpeg | ImageFormat::Png => {
            let mut decoder = reader.into_decoder().map_err(|e| {
                tracing::warn!("Upload failed: {e}");
                "could not read image".to_string()
            })?;
            let orientation = decoder.orientation().map_err(|e| {
                tracing::warn!("Upload failed: {e}");
                "could not read image".to_string()
            })?;
            let mut img = DynamicImage::from_decoder(decoder).map_err(|e| {
                tracing::warn!("Upload failed: {e}");
                "could not decode image".to_string()
            })?;
            img.apply_orientation(orientation);

            if img.width() > MAX_DIMENSION || img.height() > MAX_DIMENSION {
                img = img.resize(
                    MAX_DIMENSION,
                    MAX_DIMENSION,
                    image::imageops::FilterType::Lanczos3,
                );
            }

            let (extension, mime_type) = if format == ImageFormat::Jpeg {
                ("jpg", "image/jpeg")
            } else {
                ("png", "image/png")
            };
            let small = if img.width() > SMALL_WIDTH {
                let small =
                    img.resize(SMALL_WIDTH, u32::MAX, image::imageops::FilterType::Lanczos3);
                Some(encode(&small, format)?)
            } else {
                None
            };

            Ok(ProcessedImage {
                bytes: encode(&img, format)?,
                extension,
                mime_type,
                width: img.width(),
                height: img.height(),
                small,
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
    let name = crate::app::security::random_hex(12);
    let filename = format!("{name}.{}", image.extension);
    let small_filename = image
        .small
        .as_ref()
        .map(|_| format!("{name}-{SMALL_WIDTH}.{}", image.extension));

    tokio::fs::create_dir_all(UPLOAD_DIR).await.map_err(|e| {
        tracing::warn!("Upload failed: {e}");
        "could not create upload folder".to_string()
    })?;
    let files = std::iter::once((&filename, &image.bytes))
        .chain(small_filename.as_ref().zip(image.small.as_ref()));
    for (name, bytes) in files {
        tokio::fs::write(PathBuf::from(UPLOAD_DIR).join(name), bytes)
            .await
            .map_err(|e| {
                tracing::warn!("Upload failed: {e}");
                "could not save file".to_string()
            })?;
    }

    let original_name: String = original_name.chars().take(200).collect();
    sqlx::query(
        "INSERT INTO media (filename, small_filename, original_name, mime_type, size_bytes,
                            width, height)
         VALUES (?, ?, ?, ?, ?, ?, ?)",
    )
    .bind(&filename)
    .bind(&small_filename)
    .bind(&original_name)
    .bind(image.mime_type)
    .bind(image.bytes.len() as i64)
    .bind(image.width as i64)
    .bind(image.height as i64)
    .execute(&state.pool)
    .await
    .map_err(|e| {
        tracing::error!("Failed to record upload: {e}");
        "could not save to the media library".to_string()
    })?;

    Ok(())
}
