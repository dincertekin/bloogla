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
/// WebP quality for photos: looks like the JPEG at quality 85, but smaller.
const WEBP_QUALITY: f32 = 80.0;

/// Media library page template.
#[derive(Template)]
#[template(path = "media.html")]
pub struct MediaTemplate {
    pub blog_name: String,
    pub me: CurrentUser,
    /// The site's address, for "Copy link".
    pub base_url: String,
    pub media: Vec<MediaView>,
    pub error: Option<String>,
}

/// A media library entry prepared for display.
pub struct MediaView {
    pub id: i64,
    pub url: String,
    /// A smaller copy when there is one, for thumbnails.
    pub thumb_url: String,
    pub original_name: String,
    /// The description typed in the library (may be empty).
    pub alt_text: String,
    /// Alt text for the picture: the description, or one guessed from the
    /// file name.
    pub alt: String,
    pub size_label: String,
    pub dimensions: String,
}

impl From<Media> for MediaView {
    fn from(m: Media) -> Self {
        let url = format!("/uploads/{}", m.filename);
        // The smallest copy there is: tiles in the library are small.
        let thumb_url = m
            .thumb_filename
            .as_ref()
            .or(m.small_filename.as_ref())
            .map_or_else(|| url.clone(), |copy| format!("/uploads/{copy}"));
        let alt = if m.alt_text.trim().is_empty() {
            alt_from_file_name(&m.original_name)
        } else {
            m.alt_text.trim().to_string()
        }
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
            alt,
            url,
            thumb_url,
            original_name: m.original_name,
            alt_text: m.alt_text,
            size_label,
            dimensions,
        }
    }
}

/// `summer-in-izmir.jpg` → `summer in izmir`.
fn alt_from_file_name(name: &str) -> String {
    let stem = name.rsplit_once('.').map_or(name, |(stem, _)| stem);
    stem.replace(['-', '_'], " ").trim().to_string()
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
    /// A copy [`THUMB_WIDTH`] pixels wide, for cards and lists on phones.
    thumb: Option<Vec<u8>>,
    thumb_webp: Option<Vec<u8>>,
    /// Smaller WebP copies of the picture and of `small`, saved next to them
    /// as `<name>.webp` and served to browsers that accept WebP (see
    /// `server/middleware.rs`). Only for JPEG and PNG, and only when smaller.
    webp: Option<Vec<u8>>,
    small_webp: Option<Vec<u8>>,
}

/// Width of the smaller copy made of wide photos.
const SMALL_WIDTH: u32 = 800;
/// Width of the smallest copy: a card on a phone is about 370 pixels wide.
const THUMB_WIDTH: u32 = 400;

/// A smaller copy of an image: its bytes in the upload's format, and its
/// WebP copy when that's smaller still.
type Copy = (Vec<u8>, Option<Vec<u8>>);

/// A copy of `img` `width` pixels wide. `None` when the image isn't wider
/// than that.
fn narrower_copy(
    img: &DynamicImage,
    format: ImageFormat,
    width: u32,
) -> Result<Option<Copy>, String> {
    if img.width() <= width {
        return Ok(None);
    }
    let copy = img.resize(width, u32::MAX, image::imageops::FilterType::Lanczos3);
    let bytes = encode(&copy, format)?;
    let webp = webp_copy(&copy, format, &bytes);
    Ok(Some((bytes, webp)))
}

/// A WebP copy of `img`, if it's smaller than `original` (its JPEG or PNG).
/// Photos (JPEG) are compressed like the JPEG is; PNGs (logos, screenshots)
/// stay pixel-exact, so text and sharp edges don't blur.
fn webp_copy(img: &DynamicImage, format: ImageFormat, original: &[u8]) -> Option<Vec<u8>> {
    let lossless = format == ImageFormat::Png;
    // The image library's own buffers always hold width × height pixels,
    // which is what the WebP encoder checks.
    let encoded = if img.color().has_alpha() {
        let rgba = img.to_rgba8();
        webp::Encoder::from_rgba(&rgba, img.width(), img.height())
            .encode_simple(lossless, WEBP_QUALITY)
    } else {
        let rgb = img.to_rgb8();
        webp::Encoder::from_rgb(&rgb, img.width(), img.height())
            .encode_simple(lossless, WEBP_QUALITY)
    };
    match encoded {
        Ok(webp) if webp.len() < original.len() => Some(webp.to_vec()),
        Ok(_) => None,
        Err(e) => {
            // The original is enough; the upload still works.
            tracing::warn!("Could not make a WebP copy: {e:?}");
            None
        }
    }
}

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
            "SELECT id, filename, small_filename, thumb_filename, original_name, alt_text,
                    size_bytes, width, height
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
        base_url: state.config.base_url.clone(),
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
            base_url: state.config.base_url.clone(),
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

        if let Err(e) = add_image(state, &original_name, data.to_vec()).await {
            errors.push(format!("{original_name}: {}", lang.t_owned(&e)));
        }
    }

    errors
}

/// Check a picture, make its smaller copies and add it to the media library.
/// Uploads and imports both come through here. Gives back its file name in
/// `uploads/`, or why it was refused (English, for `lang.t_owned`).
pub async fn add_image(
    state: &AppState,
    original_name: &str,
    data: Vec<u8>,
) -> Result<String, String> {
    let processed = tokio::task::spawn_blocking(move || process_image(&data)).await;
    let image = match processed {
        Ok(result) => result?,
        Err(e) => {
            tracing::error!("Image processing crashed: {e}");
            return Err("couldn't process this image".into());
        }
    };
    save_image(state, original_name, image).await
}

#[derive(serde::Deserialize)]
pub struct AltForm {
    #[serde(default)]
    alt_text: String,
}

/// Longest image description kept.
const MAX_ALT_CHARS: usize = 250;

/// POST /admin/media/:id/alt -> Save what a picture shows.
pub async fn update_alt(
    State(state): State<AppState>,
    Extension(me): Extension<CurrentUser>,
    Path(id): Path<i64>,
    axum::Form(form): axum::Form<AltForm>,
) -> Response {
    let alt: String = form
        .alt_text
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .chars()
        .take(MAX_ALT_CHARS)
        .collect();
    let saved = sqlx::query("UPDATE media SET alt_text = ? WHERE id = ?")
        .bind(&alt)
        .bind(id)
        .execute(&state.pool)
        .await;
    match saved {
        Ok(r) if r.rows_affected() > 0 => me.t("Saved").into_response(),
        Ok(_) => (StatusCode::NOT_FOUND, me.t("Image not found")).into_response(),
        Err(e) => {
            tracing::error!("Failed to save image description: {e}");
            let message = me.t("Couldn't save. Please try again.");
            (StatusCode::INTERNAL_SERVER_ERROR, message).into_response()
        }
    }
}

/// DELETE /admin/media/:id -> Remove a file from the library and disk.
pub async fn delete(State(state): State<AppState>, Path(id): Path<i64>) -> StatusCode {
    let files: Option<(String, Option<String>, Option<String>)> = or_log(
        sqlx::query_as(
            "DELETE FROM media WHERE id = ? RETURNING filename, small_filename, thumb_filename",
        )
        .bind(id)
        .fetch_optional(&state.pool)
        .await,
        "delete media",
    );

    match files {
        Some((filename, small, thumb)) => {
            for name in std::iter::once(filename).chain(small).chain(thumb) {
                let path = PathBuf::from(UPLOAD_DIR).join(&name);
                if let Err(e) = tokio::fs::remove_file(&path).await {
                    tracing::error!("Failed to remove upload {name}: {e}");
                }
                // Its WebP copy, if it has one.
                let _ = tokio::fs::remove_file(path.with_file_name(format!("{name}.webp"))).await;
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
/// WebP are kept as they are so animations survive, except that WebP's
/// metadata is taken out (see [`without_webp_metadata`]).
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
            let (extension, mime_type, bytes) = if format == ImageFormat::Gif {
                ("gif", "image/gif", data.to_vec())
            } else {
                let bytes = without_webp_metadata(data).ok_or_else(|| {
                    tracing::warn!("Upload failed: WebP chunks don't add up");
                    "could not read image".to_string()
                })?;
                ("webp", "image/webp", bytes)
            };
            Ok(ProcessedImage {
                bytes,
                extension,
                mime_type,
                width,
                height,
                small: None,
                thumb: None,
                thumb_webp: None,
                webp: None,
                small_webp: None,
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
            let (small, small_webp) = narrower_copy(&img, format, SMALL_WIDTH)?.unzip();
            let (thumb, thumb_webp) = narrower_copy(&img, format, THUMB_WIDTH)?.unzip();
            let bytes = encode(&img, format)?;
            let webp = webp_copy(&img, format, &bytes);

            Ok(ProcessedImage {
                bytes,
                extension,
                mime_type,
                width: img.width(),
                height: img.height(),
                small,
                thumb,
                thumb_webp: thumb_webp.flatten(),
                webp,
                small_webp: small_webp.flatten(),
            })
        }
        _ => Err("not a supported image (JPEG, PNG, GIF or WebP)".to_string()),
    }
}

/// A WebP file with only the chunks needed to show it: the picture,
/// transparency, animation frames and color profile, copied as they are.
/// Everything else goes: `EXIF` and `XMP ` (what the camera wrote: GPS
/// location, phone model, when it was taken) and any chunk an app added,
/// which readers ignore anyway. `None` if the chunks don't add up. A missing
/// padding byte after the last chunk is added rather than refused.
///
/// A WebP file is `RIFF`, its size, `WEBP`, then chunks: a 4-letter name, a
/// size, the data, and a padding byte if the size is odd. Files with
/// metadata start with a `VP8X` chunk whose flags say EXIF and XMP are
/// there; those flags are cleared too.
fn without_webp_metadata(data: &[u8]) -> Option<Vec<u8>> {
    const EXIF_FLAG: u8 = 0x08;
    const XMP_FLAG: u8 = 0x04;

    if data.len() < 12 || &data[0..4] != b"RIFF" || &data[8..12] != b"WEBP" {
        return None;
    }
    let riff_size = u32::from_le_bytes(data[4..8].try_into().ok()?) as usize;
    let end = riff_size.checked_add(8)?;
    let chunks = data.get(12..end)?;

    let mut kept = b"WEBP".to_vec();
    let mut rest = chunks;
    while !rest.is_empty() {
        let name = rest.get(0..4)?;
        let size = u32::from_le_bytes(rest.get(4..8)?.try_into().ok()?) as usize;
        let unpadded = size.checked_add(8)?;
        let padded = unpadded.checked_add(size % 2)?;
        // Some tools leave out the padding byte after the last chunk.
        let taken = if rest.len() >= padded {
            padded
        } else if rest.len() == unpadded {
            unpadded
        } else {
            return None;
        };
        let mut chunk = rest[..taken].to_vec();
        rest = &rest[taken..];
        chunk.resize(padded, 0);
        match name {
            b"VP8X" if size >= 1 => chunk[8] &= !(EXIF_FLAG | XMP_FLAG),
            b"VP8 " | b"VP8L" | b"ALPH" | b"ANIM" | b"ANMF" | b"ICCP" => {}
            _ => continue,
        }
        kept.extend_from_slice(&chunk);
    }

    let mut out = b"RIFF".to_vec();
    out.extend_from_slice(&u32::try_from(kept.len()).ok()?.to_le_bytes());
    out.extend_from_slice(&kept);
    Some(out)
}

/// Give images uploaded before Bloogla made 400-pixel copies (1.2.1 and
/// earlier) theirs, so cards on phones stay light on older sites too. Runs
/// once in the background when Bloogla starts; images that already have a
/// copy, or don't need one, are skipped.
pub async fn make_missing_thumbs(pool: sqlx::SqlitePool) {
    let missing: Vec<(i64, String)> = or_log(
        sqlx::query_as(
            "SELECT id, filename FROM media
             WHERE thumb_filename IS NULL AND width > ?
               AND mime_type IN ('image/jpeg', 'image/png')",
        )
        .bind(THUMB_WIDTH)
        .fetch_all(&pool)
        .await,
        "find images without a small copy",
    );
    let mut made = 0;
    for (id, filename) in missing {
        let name = filename.clone();
        let copy = tokio::task::spawn_blocking(move || thumb_for(&name)).await;
        let Ok(Ok((thumb_filename, (bytes, webp)))) = copy else {
            tracing::warn!("Couldn't make a small copy of {filename}");
            continue;
        };
        let folder = PathBuf::from(UPLOAD_DIR);
        let written = tokio::fs::write(folder.join(&thumb_filename), &bytes).await;
        if written.is_err() {
            tracing::warn!("Couldn't save the small copy of {filename}");
            continue;
        }
        if let Some(webp) = webp {
            let _ = tokio::fs::write(folder.join(format!("{thumb_filename}.webp")), webp).await;
        }
        let saved = sqlx::query("UPDATE media SET thumb_filename = ? WHERE id = ?")
            .bind(&thumb_filename)
            .bind(id)
            .execute(&pool)
            .await;
        if saved.is_ok() {
            made += 1;
        }
    }
    if made > 0 {
        tracing::info!("Made smaller copies of {made} images for phones");
    }
}

/// The 400-pixel copy of an uploaded image: its file name, its bytes and
/// its WebP copy. Uploads are already upright and size-limited.
fn thumb_for(filename: &str) -> Result<(String, Copy), String> {
    let data =
        std::fs::read(PathBuf::from(UPLOAD_DIR).join(filename)).map_err(|e| e.to_string())?;
    let format = image::guess_format(&data).map_err(|e| e.to_string())?;
    let img = image::load_from_memory_with_format(&data, format).map_err(|e| e.to_string())?;
    let (bytes, webp) =
        narrower_copy(&img, format, THUMB_WIDTH)?.ok_or("not wider than a small copy")?;
    let (stem, extension) = filename.rsplit_once('.').ok_or("no file ending")?;
    Ok((format!("{stem}-{THUMB_WIDTH}.{extension}"), (bytes, webp)))
}

/// Write a processed image under a random name and record it in the library.
/// Gives back the file name.
async fn save_image(
    state: &AppState,
    original_name: &str,
    image: ProcessedImage,
) -> Result<String, String> {
    let name = crate::app::security::random_hex(12);
    let filename = format!("{name}.{}", image.extension);
    let small_filename = image
        .small
        .as_ref()
        .map(|_| format!("{name}-{SMALL_WIDTH}.{}", image.extension));
    let thumb_filename = image
        .thumb
        .as_ref()
        .map(|_| format!("{name}-{THUMB_WIDTH}.{}", image.extension));

    tokio::fs::create_dir_all(UPLOAD_DIR).await.map_err(|e| {
        tracing::warn!("Upload failed: {e}");
        "could not create upload folder".to_string()
    })?;
    let webp_filename = format!("{filename}.webp");
    let small_webp_filename = small_filename.as_ref().map(|name| format!("{name}.webp"));
    let thumb_webp_filename = thumb_filename.as_ref().map(|name| format!("{name}.webp"));
    let files = std::iter::once((&filename, &image.bytes))
        .chain(small_filename.as_ref().zip(image.small.as_ref()))
        .chain(thumb_filename.as_ref().zip(image.thumb.as_ref()))
        .chain(image.webp.as_ref().map(|bytes| (&webp_filename, bytes)))
        .chain(small_webp_filename.as_ref().zip(image.small_webp.as_ref()))
        .chain(thumb_webp_filename.as_ref().zip(image.thumb_webp.as_ref()));
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
        "INSERT INTO media (filename, small_filename, thumb_filename, original_name, mime_type,
                            size_bytes, width, height)
         VALUES (?, ?, ?, ?, ?, ?, ?, ?)",
    )
    .bind(&filename)
    .bind(&small_filename)
    .bind(&thumb_filename)
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

    Ok(filename)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A small lossless WebP, and the same picture as `RIFF` chunks.
    fn webp() -> (DynamicImage, Vec<u8>) {
        let img = DynamicImage::ImageRgba8(image::RgbaImage::from_fn(6, 4, |x, y| {
            image::Rgba([x as u8 * 40, y as u8 * 60, 90, 255])
        }));
        let mut bytes = Vec::new();
        img.write_with_encoder(image::codecs::webp::WebPEncoder::new_lossless(&mut bytes))
            .unwrap();
        (img, bytes)
    }

    fn chunk(name: &[u8; 4], data: &[u8]) -> Vec<u8> {
        let mut out = name.to_vec();
        out.extend_from_slice(&(data.len() as u32).to_le_bytes());
        out.extend_from_slice(data);
        if data.len() % 2 == 1 {
            out.push(0);
        }
        out
    }

    /// `body` (the `VP8L` chunk of [`webp`]) with a `VP8X` header saying
    /// EXIF and XMP follow, and those two chunks, like a phone camera writes.
    fn with_location(body: &[u8]) -> Vec<u8> {
        let mut vp8x = vec![0x08 | 0x04, 0, 0, 0];
        vp8x.extend_from_slice(&[5, 0, 0, 3, 0, 0]); // 6 x 4, stored minus one
        let mut chunks = b"WEBP".to_vec();
        chunks.extend(chunk(b"VP8X", &vp8x));
        chunks.extend_from_slice(body);
        // An odd size, so the padding byte is handled too.
        chunks.extend(chunk(b"EXIF", b"MM\0*GPS 41.0128N 28.9800E Pixel 9"));
        chunks.extend(chunk(b"XMP ", b"<x:xmpmeta>exif:GPSLatitude</x:xmpmeta>"));
        let mut file = b"RIFF".to_vec();
        file.extend_from_slice(&(chunks.len() as u32).to_le_bytes());
        file.extend(chunks);
        file
    }

    fn contains(haystack: &[u8], needle: &[u8]) -> bool {
        haystack.windows(needle.len()).any(|w| w == needle)
    }

    #[test]
    fn webp_uploads_lose_their_location() {
        let (img, plain) = webp();
        let upload = with_location(&plain[12..]);
        assert!(contains(&upload, b"41.0128N"));

        let saved = process_image(&upload).expect("a valid WebP");
        assert_eq!(saved.mime_type, "image/webp");
        for gone in [
            &b"EXIF"[..],
            b"XMP ",
            b"41.0128N",
            b"Pixel 9",
            b"GPSLatitude",
        ] {
            assert!(
                !contains(&saved.bytes, gone),
                "{} is still in the file",
                String::from_utf8_lossy(gone)
            );
        }
        // The VP8X flags no longer promise EXIF or XMP.
        assert_eq!(saved.bytes[20] & (0x08 | 0x04), 0);
        // The picture itself is untouched.
        assert!(contains(&saved.bytes, &plain[12..]));
        let back = image::load_from_memory(&saved.bytes).expect("still a WebP");
        assert_eq!(back.to_rgba8(), img.to_rgba8());
    }

    #[test]
    fn webp_without_metadata_is_kept_as_is() {
        let (_, plain) = webp();
        assert_eq!(process_image(&plain).expect("valid").bytes, plain);
    }

    #[test]
    fn webp_without_the_last_padding_byte_is_accepted() {
        let (img, plain) = webp();
        // An odd-sized metadata chunk last, with its padding byte left out.
        let mut upload = with_location(&plain[12..]);
        upload.pop();
        let riff_size = (upload.len() - 8) as u32;
        upload[4..8].copy_from_slice(&riff_size.to_le_bytes());

        let saved = process_image(&upload).expect("still a valid WebP");
        assert!(!contains(&saved.bytes, b"GPSLatitude"));
        let back = image::load_from_memory(&saved.bytes).expect("still a WebP");
        assert_eq!(back.to_rgba8(), img.to_rgba8());
    }

    #[test]
    fn chunks_an_app_added_are_removed_too() {
        let (img, plain) = webp();
        let mut upload = plain.clone();
        upload.extend(chunk(b"ZZZZ", b"lat=41.0128;lon=28.98"));
        let riff_size = (upload.len() - 8) as u32;
        upload[4..8].copy_from_slice(&riff_size.to_le_bytes());

        let saved = process_image(&upload).expect("a valid WebP");
        assert_eq!(saved.bytes, plain);
        let back = image::load_from_memory(&saved.bytes).expect("still a WebP");
        assert_eq!(back.to_rgba8(), img.to_rgba8());
    }

    #[test]
    fn a_picture_chunk_last_gets_its_missing_padding_byte() {
        // The picture chunk is usually last, so that's where tools leave
        // the padding byte out. Find a picture whose chunk has an odd size.
        let (img, plain) = (1..40)
            .map(|width| {
                let img = DynamicImage::ImageRgba8(image::RgbaImage::from_fn(width, 3, |x, y| {
                    image::Rgba([x as u8 * 6, y as u8 * 70, 200, 255])
                }));
                let mut bytes = Vec::new();
                img.write_with_encoder(image::codecs::webp::WebPEncoder::new_lossless(&mut bytes))
                    .unwrap();
                (img, bytes)
            })
            .find(|(_, bytes)| u32::from_le_bytes(bytes[16..20].try_into().unwrap()) % 2 == 1)
            .expect("some width gives an odd size");
        assert_eq!(*plain.last().unwrap(), 0, "the encoder pads it");
        let mut upload = plain[..plain.len() - 1].to_vec();
        let riff_size = (upload.len() - 8) as u32;
        upload[4..8].copy_from_slice(&riff_size.to_le_bytes());

        let saved = process_image(&upload).expect("still a valid WebP");
        assert_eq!(saved.bytes, plain, "the padding byte is back");
        let back = image::load_from_memory(&saved.bytes).expect("still a WebP");
        assert_eq!(back.to_rgba8(), img.to_rgba8());
    }

    #[test]
    fn webp_with_chunks_that_dont_add_up_is_refused() {
        let (_, plain) = webp();
        let upload = with_location(&plain[12..]);
        // Cut inside the EXIF chunk, but keep the RIFF size claiming more.
        assert!(without_webp_metadata(&upload[..upload.len() - 20]).is_none());
    }
}
