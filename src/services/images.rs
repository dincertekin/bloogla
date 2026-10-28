//! Pictures for the media library: checking an upload by its content (not
//! its name), making it ready for the web, and saving it with its smaller
//! copies. The Media screen (`handlers/admin/media.rs`) and the WordPress
//! import both add pictures through [`add_image`].
//!
//! Every picture is resized for the web and stripped of metadata such as GPS
//! location before it's saved.

use crate::app::state::AppState;
use crate::db::media::{self, NewMedia};

use image::codecs::jpeg::JpegEncoder;
use image::codecs::png::{CompressionType, PngEncoder};
use image::{DynamicImage, ImageDecoder, ImageFormat, ImageReader, Limits};
use std::io::Cursor;
use std::path::PathBuf;

/// Folder uploads are saved in, served at `/uploads`.
pub const UPLOAD_DIR: &str = "uploads";
/// The largest image accepted (each file; several can be uploaded at once).
pub const MAX_UPLOAD_BYTES: usize = 20 * 1024 * 1024;
/// Animations (GIF, animated WebP) can't be made smaller like photos; bigger
/// ones than this make pages slow, so they're refused.
const MAX_ANIMATION_BYTES: usize = 5 * 1024 * 1024;

/// Why a picture was refused (English; `lang.t_owned` translates them).
const TOO_MANY_PIXELS: &str =
    "more than 12,000 pixels wide or tall: make it smaller and upload it again";
const ANIMATION_TOO_LARGE: &str = "an animation larger than 5 MB makes pages slow to load: make it shorter or smaller, or post it as a video on YouTube or Vimeo";
/// Larger photos are scaled down to fit this box.
const MAX_DIMENSION: u32 = 2000;
const JPEG_QUALITY: u8 = 85;
/// WebP quality for photos: looks like the JPEG at quality 85, but smaller.
const WEBP_QUALITY: f32 = 80.0;

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

/// How a picture is saved: JPEG and PNG (with a WebP copy next to them for
/// browsers that take WebP), or WebP itself.
#[derive(Clone, Copy, PartialEq)]
enum Output {
    Jpeg,
    Png,
    /// `true` keeps every pixel exact (graphics), `false` compresses like a
    /// photo.
    WebP(bool),
}

/// A smaller copy of an image: its bytes, and for JPEG and PNG its WebP copy
/// when that's smaller still.
type Copy = (Vec<u8>, Option<Vec<u8>>);

/// A copy of `img` `width` pixels wide. `None` when the image isn't wider
/// than that.
fn narrower_copy(img: &DynamicImage, output: Output, width: u32) -> Result<Option<Copy>, String> {
    if img.width() <= width {
        return Ok(None);
    }
    let copy = img.resize(width, u32::MAX, image::imageops::FilterType::Lanczos3);
    let bytes = encode(&copy, output)?;
    let webp = webp_copy(&copy, output, &bytes);
    Ok(Some((bytes, webp)))
}

/// A WebP copy of a JPEG or PNG `img`, if it's smaller than `original`.
fn webp_copy(img: &DynamicImage, output: Output, original: &[u8]) -> Option<Vec<u8>> {
    let lossless = match output {
        Output::Jpeg => false,
        Output::Png => true,
        Output::WebP(_) => return None, // already WebP
    };
    match encode_webp(img, lossless) {
        Ok(webp) if webp.len() < original.len() => Some(webp),
        Ok(_) => None,
        Err(e) => {
            // The original is enough; the upload still works.
            tracing::warn!("Could not make a WebP copy: {e}");
            None
        }
    }
}

/// `img` as WebP. Photos (`lossless` false) are compressed like the JPEG is;
/// graphics stay pixel-exact, so text and sharp edges don't blur.
fn encode_webp(img: &DynamicImage, lossless: bool) -> Result<Vec<u8>, String> {
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
    encoded
        .map(|webp| webp.to_vec())
        .map_err(|e| format!("{e:?}"))
}

/// Encode `img` as `output`.
fn encode(img: &DynamicImage, output: Output) -> Result<Vec<u8>, String> {
    let mut bytes = Vec::new();
    let result = match output {
        Output::Jpeg => {
            let encoder = JpegEncoder::new_with_quality(&mut bytes, JPEG_QUALITY);
            img.to_rgb8().write_with_encoder(encoder)
        }
        Output::Png => {
            // Best compression: uploads are saved once and downloaded many times.
            let encoder = PngEncoder::new_with_quality(
                &mut bytes,
                CompressionType::Best,
                image::codecs::png::FilterType::Adaptive,
            );
            img.write_with_encoder(encoder)
        }
        Output::WebP(lossless) => {
            return encode_webp(img, lossless).map_err(|e| {
                tracing::warn!("Upload failed: {e}");
                "could not encode image".to_string()
            })
        }
    };
    result.map_err(|e| {
        tracing::warn!("Upload failed: {e}");
        "could not encode image".to_string()
    })?;
    Ok(bytes)
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

/// Check an upload by its content (not its name) and make it ready for the
/// web.
///
/// Still pictures (JPEG, PNG, GIF, WebP) are decoded, turned upright, scaled
/// down to [`MAX_DIMENSION`], re-encoded (which also drops metadata such as
/// GPS location) and given smaller copies for phones. Animations (GIF,
/// animated WebP) are kept as they are, up to [`MAX_ANIMATION_BYTES`]; a
/// WebP animation loses its metadata (see [`without_webp_metadata`]).
fn process_image(data: &[u8]) -> Result<ProcessedImage, String> {
    let format = image::guess_format(data)
        .map_err(|_| "not a supported image (JPEG, PNG, GIF or WebP)".to_string())?;
    if !matches!(
        format,
        ImageFormat::Jpeg | ImageFormat::Png | ImageFormat::Gif | ImageFormat::WebP
    ) {
        return Err("not a supported image (JPEG, PNG, GIF or WebP)".to_string());
    }

    let reader = || {
        let mut limits = Limits::default();
        limits.max_image_width = Some(12_000);
        limits.max_image_height = Some(12_000);
        let mut reader = ImageReader::with_format(Cursor::new(data), format);
        reader.limits(limits);
        reader
    };
    // Its size first: a file can claim a huge picture to fill the memory.
    let (width, height) = reader().into_dimensions().map_err(unreadable)?;
    if is_animation(data, format) {
        return keep_animation(data, format, width, height);
    }

    let mut decoder = reader().into_decoder().map_err(unreadable)?;
    let orientation = decoder.orientation().map_err(unreadable)?;
    let mut img = DynamicImage::from_decoder(decoder).map_err(unreadable)?;
    img.apply_orientation(orientation);
    let resized = img.width() > MAX_DIMENSION || img.height() > MAX_DIMENSION;
    if resized {
        img = img.resize(
            MAX_DIMENSION,
            MAX_DIMENSION,
            image::imageops::FilterType::Lanczos3,
        );
    }

    // A still GIF is saved like a PNG (same pixels, smaller file).
    let (output, extension, mime_type) = match format {
        ImageFormat::Jpeg => (Output::Jpeg, "jpg", "image/jpeg"),
        ImageFormat::WebP => (Output::WebP(is_lossless_webp(data)), "webp", "image/webp"),
        _ => (Output::Png, "png", "image/png"),
    };
    let (small, small_webp) = narrower_copy(&img, output, SMALL_WIDTH)?.unzip();
    let (thumb, thumb_webp) = narrower_copy(&img, output, THUMB_WIDTH)?.unzip();
    // A WebP that didn't need resizing keeps its own bytes (without its
    // metadata): encoding it again would only lose quality.
    let bytes = match output {
        Output::WebP(_) if !resized => without_webp_metadata(data).ok_or_else(|| {
            tracing::warn!("Upload failed: WebP chunks don't add up");
            "could not read image".to_string()
        })?,
        _ => encode(&img, output)?,
    };
    let webp = webp_copy(&img, output, &bytes);

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

/// What a picture that can't be read is called: too big, or broken.
fn unreadable(error: image::ImageError) -> String {
    tracing::warn!("Upload failed: {error}");
    if matches!(error, image::ImageError::Limits(_)) {
        TOO_MANY_PIXELS.to_string()
    } else {
        "could not read image".to_string()
    }
}

/// Whether a GIF or WebP moves (has more than one frame).
fn is_animation(data: &[u8], format: ImageFormat) -> bool {
    use image::AnimationDecoder;
    match format {
        ImageFormat::Gif => image::codecs::gif::GifDecoder::new(Cursor::new(data))
            .map(|gif| gif.into_frames().take(2).count() > 1)
            .unwrap_or(false),
        ImageFormat::WebP => image::codecs::webp::WebPDecoder::new(Cursor::new(data))
            .map(|webp| webp.has_animation())
            .unwrap_or(false),
        _ => false,
    }
}

/// An animation, kept as it is (resizing every frame would be slow and
/// lose quality), if it's small enough to load quickly.
fn keep_animation(
    data: &[u8],
    format: ImageFormat,
    width: u32,
    height: u32,
) -> Result<ProcessedImage, String> {
    if data.len() > MAX_ANIMATION_BYTES {
        return Err(ANIMATION_TOO_LARGE.to_string());
    }
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

/// Whether a WebP is stored pixel-exact (`VP8L`, used for graphics) rather
/// than compressed like a photo (`VP8 `): its copies are made the same way.
fn is_lossless_webp(data: &[u8]) -> bool {
    // Chunks after the 12-byte header: a 4-letter name, a size, the data.
    let mut at = 12;
    while at + 8 <= data.len() {
        let name = &data[at..at + 4];
        if name == b"VP8L" {
            return true;
        }
        if name == b"VP8 " {
            return false;
        }
        let size =
            u32::from_le_bytes([data[at + 4], data[at + 5], data[at + 6], data[at + 7]]) as usize;
        at += 8 + size + size % 2;
    }
    false
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
    let missing = media::without_thumb(&pool, THUMB_WIDTH).await;
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
        if media::set_thumb(&pool, id, &thumb_filename).await.is_ok() {
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
    let output = if format == ImageFormat::Jpeg {
        Output::Jpeg
    } else {
        Output::Png
    };
    let img = image::load_from_memory_with_format(&data, format).map_err(|e| e.to_string())?;
    let (bytes, webp) =
        narrower_copy(&img, output, THUMB_WIDTH)?.ok_or("not wider than a small copy")?;
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
    let new_media = NewMedia {
        filename: &filename,
        small_filename: small_filename.as_deref(),
        thumb_filename: thumb_filename.as_deref(),
        original_name: &original_name,
        mime_type: image.mime_type,
        size_bytes: image.bytes.len() as i64,
        width: Some(image.width as i64),
        height: Some(image.height as i64),
    };
    media::add(&state.pool, &new_media).await.map_err(|e| {
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
