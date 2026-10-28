//! Short videos for the media library: checking an upload is really an MP4 or
//! WebM video, removing where an MP4 was filmed, and adding it to Media.
//!
//! Videos are kept as they are: making them smaller would need a video
//! converter (ffmpeg), far bigger than Bloogla itself. Long videos belong on
//! YouTube or Vimeo.

use crate::app::state::AppState;
use crate::db::media::{self, NewMedia};
use crate::services::images::UPLOAD_DIR;

use std::io::{Read, Seek, SeekFrom, Write};
use std::path::Path;

/// The largest video accepted.
pub const MAX_VIDEO_BYTES: u64 = 100 * 1024 * 1024;

/// Why a video was refused (English; `lang.t_owned` translates them).
pub const VIDEO_TOO_LARGE: &str =
    "a video larger than 100 MB: make it shorter, or post it on YouTube or Vimeo";
const NOT_A_VIDEO: &str = "not a supported video: save it as MP4 or WebM";
const UNREADABLE: &str = "could not read video";

/// What kind of video a file is, from its first bytes: `(extension, type)`.
/// `None` for anything else (pictures included).
pub fn kind_of(start: &[u8]) -> Option<(&'static str, &'static str)> {
    if is_mp4(start) {
        Some(("mp4", "video/mp4"))
    } else if is_webm(start) {
        Some(("webm", "video/webm"))
    } else {
        None
    }
}

/// Whether a file starts like a file of the MP4 family (`ftyp` and a brand
/// name), even one browsers can't play, so it's refused as a video rather
/// than as a broken picture.
pub fn looks_like_mp4_family(start: &[u8]) -> bool {
    start.get(4..8) == Some(b"ftyp")
}

/// An MP4 starts with an `ftyp` box naming its brand. Pictures (HEIC, AVIF)
/// and QuickTime films (`qt  `, which most browsers can't play) use the same
/// box with other brands, so only these are accepted.
fn is_mp4(start: &[u8]) -> bool {
    const BRANDS: [&[u8; 4]; 12] = [
        b"isom", b"iso2", b"iso3", b"iso4", b"iso5", b"iso6", b"mp41", b"mp42", b"avc1", b"M4V ",
        b"dash", b"MSNV",
    ];
    looks_like_mp4_family(start)
        && start
            .get(8..12)
            .is_some_and(|brand| BRANDS.iter().any(|b| *b == brand))
}

/// A WebM starts with the EBML header (`1A 45 DF A3`) whose document type
/// says `webm` (other Matroska files say `matroska`).
fn is_webm(start: &[u8]) -> bool {
    let head = &start[..start.len().min(64)];
    head.starts_with(&[0x1A, 0x45, 0xDF, 0xA3]) && head.windows(4).any(|w| w == b"webm")
}

/// Add a video saved at `temporary` (in `uploads/`) to the media library,
/// under a random name. Gives back its file name, or why it was refused
/// (English, for `lang.t_owned`). The temporary file is gone either way.
pub async fn add_video(
    state: &AppState,
    original_name: &str,
    temporary: &Path,
    (extension, mime_type): (&'static str, &'static str),
) -> Result<String, String> {
    let checked = if extension == "mp4" {
        let path = temporary.to_path_buf();
        tokio::task::spawn_blocking(move || hide_mp4_metadata(&path))
            .await
            .unwrap_or(Err(UNREADABLE))
    } else {
        Ok(())
    };
    let size = tokio::fs::metadata(temporary).await.map(|m| m.len());
    let (Ok(()), Ok(size)) = (checked, size) else {
        let _ = tokio::fs::remove_file(temporary).await;
        return Err(UNREADABLE.to_string());
    };

    let filename = format!("{}.{extension}", crate::app::security::random_hex(12));
    if let Err(e) = tokio::fs::rename(temporary, Path::new(UPLOAD_DIR).join(&filename)).await {
        tracing::warn!("Upload failed: {e}");
        let _ = tokio::fs::remove_file(temporary).await;
        return Err("could not save file".to_string());
    }
    let original_name: String = original_name.chars().take(200).collect();
    let new_media = NewMedia {
        filename: &filename,
        small_filename: None,
        thumb_filename: None,
        original_name: &original_name,
        mime_type,
        size_bytes: size as i64,
        width: None,
        height: None,
    };
    if let Err(e) = media::add(&state.pool, &new_media).await {
        tracing::error!("Failed to record upload: {e}");
        let _ = tokio::fs::remove_file(Path::new(UPLOAD_DIR).join(&filename)).await;
        return Err("could not save to the media library".to_string());
    }
    Ok(filename)
}

/// The refusal for something that isn't an MP4 or WebM.
pub fn not_a_video() -> String {
    NOT_A_VIDEO.to_string()
}

/// Where an MP4 was filmed, the phone it was filmed with and similar notes
/// are kept in `udta` and `meta` boxes. Each is turned into a `free` box
/// (empty space players skip) of the same size: renamed, and its contents
/// overwritten with zeros, so the notes are gone and nothing else in the file
/// moves. The video itself stays exactly as it was. Also checks the file's
/// boxes add up.
///
/// An MP4 is a list of boxes: a 4-byte size, a 4-letter name, then the box's
/// data. `moov` (the description of the video) holds `trak` boxes (one per
/// track), and both may hold `udta` and `meta`.
fn hide_mp4_metadata(path: &Path) -> Result<(), &'static str> {
    /// `moov` is read into memory; a real one is a few megabytes at most.
    const MAX_MOOV_BYTES: u64 = 32 * 1024 * 1024;
    let mut file = std::fs::OpenOptions::new()
        .read(true)
        .write(true)
        .open(path)
        .map_err(|_| UNREADABLE)?;
    let length = file.metadata().map_err(|_| UNREADABLE)?.len();
    let (mut at, mut found_moov) = (0u64, false);
    while at < length {
        let (header, size, name) = box_header_at(&mut file, at, length)?;
        match &name {
            b"moov" => {
                if size > MAX_MOOV_BYTES {
                    return Err(UNREADABLE);
                }
                let mut moov = vec![0; size as usize];
                file.seek(SeekFrom::Start(at)).map_err(|_| UNREADABLE)?;
                file.read_exact(&mut moov).map_err(|_| UNREADABLE)?;
                hide_in(&mut moov[header as usize..], 0)?;
                file.seek(SeekFrom::Start(at)).map_err(|_| UNREADABLE)?;
                file.write_all(&moov).map_err(|_| UNREADABLE)?;
                found_moov = true;
            }
            b"udta" | b"meta" => {
                if size > MAX_MOOV_BYTES {
                    return Err(UNREADABLE);
                }
                let mut blank = vec![0; size as usize];
                blank[..header as usize].copy_from_slice(&header_bytes(header, size));
                file.seek(SeekFrom::Start(at)).map_err(|_| UNREADABLE)?;
                file.write_all(&blank).map_err(|_| UNREADABLE)?;
            }
            _ => {}
        }
        at += size;
    }
    found_moov.then_some(()).ok_or(UNREADABLE)
}

/// The header of a `free` box `size` bytes long, as long as the one it
/// replaces (`header` bytes).
fn header_bytes(header: u64, size: u64) -> Vec<u8> {
    if header == 16 {
        [&1u32.to_be_bytes()[..], b"free", &size.to_be_bytes()].concat()
    } else {
        [&(size as u32).to_be_bytes()[..], b"free"].concat()
    }
}

/// The header length, size and name of the box at `at` in a file `length`
/// bytes long. A size of 1 means a 64-bit size follows; 0 means "to the end".
fn box_header_at(
    file: &mut std::fs::File,
    at: u64,
    length: u64,
) -> Result<(u64, u64, [u8; 4]), &'static str> {
    let mut head = [0u8; 16];
    let available = (length - at).min(16) as usize;
    if available < 8 {
        return Err(UNREADABLE);
    }
    file.seek(SeekFrom::Start(at)).map_err(|_| UNREADABLE)?;
    file.read_exact(&mut head[..available])
        .map_err(|_| UNREADABLE)?;
    let name = [head[4], head[5], head[6], head[7]];
    let (header, size) = match u32::from_be_bytes([head[0], head[1], head[2], head[3]]) {
        0 => (8, length - at),
        1 if available == 16 => {
            let mut wide = [0u8; 8];
            wide.copy_from_slice(&head[8..16]);
            (16, u64::from_be_bytes(wide))
        }
        1 => return Err(UNREADABLE),
        size => (8, u64::from(size)),
    };
    if size < header || size > length - at {
        return Err(UNREADABLE);
    }
    Ok((header, size, name))
}

/// Rename the `udta` and `meta` boxes among `boxes` (and inside its `trak`
/// boxes) to `free`. `depth` stops files built to nest forever.
fn hide_in(boxes: &mut [u8], depth: u8) -> Result<(), &'static str> {
    if depth > 2 {
        return Ok(());
    }
    let mut at = 0usize;
    while at < boxes.len() {
        let rest = &boxes[at..];
        let size = rest
            .get(0..4)
            .map(|b| u32::from_be_bytes([b[0], b[1], b[2], b[3]]) as usize)
            .ok_or(UNREADABLE)?;
        // Boxes inside `moov` are small: no 64-bit or "to the end" sizes.
        if size < 8 || size > rest.len() {
            return Err(UNREADABLE);
        }
        match &rest[4..8] {
            b"udta" | b"meta" => {
                boxes[at + 4..at + 8].copy_from_slice(b"free");
                boxes[at + 8..at + size].fill(0);
            }
            b"trak" => hide_in(&mut boxes[at + 8..at + size], depth + 1)?,
            _ => {}
        }
        at += size;
    }
    Ok(())
}

#[cfg(test)]
pub mod tests {
    use super::*;

    /// One MP4 box.
    pub fn mp4_box(name: &[u8; 4], data: &[u8]) -> Vec<u8> {
        let mut out = ((data.len() + 8) as u32).to_be_bytes().to_vec();
        out.extend_from_slice(name);
        out.extend_from_slice(data);
        out
    }

    /// A small MP4 like a phone writes: where it was filmed in `moov/udta`
    /// and `moov/meta`, and the phone's name in a track's `udta`.
    pub fn phone_mp4() -> Vec<u8> {
        let track = mp4_box(
            b"trak",
            &[mp4_box(b"tkhd", &[0; 12]), mp4_box(b"udta", b"Pixel 9 Pro")].concat(),
        );
        let moov = mp4_box(
            b"moov",
            &[
                mp4_box(b"mvhd", &[0; 12]),
                track,
                mp4_box(b"udta", b"\xa9xyz+41.0128+028.9800/"),
                mp4_box(b"meta", b"com.apple.quicktime.location.ISO6709"),
            ]
            .concat(),
        );
        [
            mp4_box(b"ftyp", b"isom\0\0\x02\0isomiso2mp41"),
            moov,
            mp4_box(b"mdat", &[7; 64]),
        ]
        .concat()
    }

    #[test]
    fn videos_are_recognised_by_their_content() {
        assert_eq!(kind_of(&phone_mp4()), Some(("mp4", "video/mp4")));
        let webm = [
            &[0x1A, 0x45, 0xDF, 0xA3, 0x9F, 0x42, 0x82, 0x84][..],
            b"webm",
        ]
        .concat();
        assert_eq!(kind_of(&webm), Some(("webm", "video/webm")));
        let quicktime = mp4_box(b"ftyp", b"qt  \0\0\0\0");
        assert_eq!(kind_of(&quicktime), None);
        assert!(looks_like_mp4_family(&quicktime));
        let heic = mp4_box(b"ftyp", b"heic\0\0\0\0mif1");
        assert_eq!(kind_of(&heic), None);
        assert_eq!(kind_of(b"\x89PNG\r\n\x1a\n"), None);
    }

    #[test]
    fn where_a_video_was_filmed_is_removed_and_nothing_else_moves() {
        let original = phone_mp4();
        let path = std::env::temp_dir().join(format!(
            "bloogla-{}.mp4",
            crate::app::security::random_hex(6)
        ));
        std::fs::write(&path, &original).unwrap();
        hide_mp4_metadata(&path).unwrap();
        let saved = std::fs::read(&path).unwrap();
        std::fs::remove_file(&path).unwrap();

        assert_eq!(saved.len(), original.len());
        let has = |text: &[u8]| saved.windows(text.len()).any(|w| w == text);
        assert!(!has(b"udta") && !has(b"meta"));
        // The notes themselves are gone, not just hidden.
        assert!(!has(b"+41.0128") && !has(b"Pixel 9") && !has(b"ISO6709"));
        assert!(has(b"free") && has(b"mdat") && has(b"tkhd"));
        // The video itself (`mdat`) is untouched.
        let video = &original[original.len() - 72..];
        assert_eq!(&saved[saved.len() - 72..], video);
    }

    #[test]
    fn notes_outside_moov_are_blanked_too() {
        let original = [
            mp4_box(b"ftyp", b"isom\0\0\x02\0"),
            mp4_box(b"udta", b"\xa9xyz+41.0128+028.9800/"),
            mp4_box(b"moov", &mp4_box(b"mvhd", &[0; 12])),
            mp4_box(b"mdat", &[7; 64]),
        ]
        .concat();
        let path = std::env::temp_dir().join(format!(
            "bloogla-{}.mp4",
            crate::app::security::random_hex(6)
        ));
        std::fs::write(&path, &original).unwrap();
        hide_mp4_metadata(&path).unwrap();
        let saved = std::fs::read(&path).unwrap();
        std::fs::remove_file(&path).unwrap();
        assert_eq!(saved.len(), original.len());
        assert!(!saved.windows(8).any(|w| w == b"+41.0128"));
        assert!(saved.windows(4).any(|w| w == b"free"));
    }

    #[test]
    fn an_mp4_whose_boxes_dont_add_up_is_refused() {
        let mut broken = phone_mp4();
        broken.truncate(broken.len() - 10);
        let path = std::env::temp_dir().join(format!(
            "bloogla-{}.mp4",
            crate::app::security::random_hex(6)
        ));
        std::fs::write(&path, &broken).unwrap();
        assert!(hide_mp4_metadata(&path).is_err());
        std::fs::remove_file(&path).unwrap();
    }
}
