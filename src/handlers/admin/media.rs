//! The Media screen: uploading, listing, describing and deleting pictures and
//! short videos, and the chooser inside the post editor. The files themselves
//! are checked in `services/images.rs` and `services/videos.rs`.

use crate::app::models::{CurrentUser, Media};
use crate::app::state::AppState;
use crate::db::{media, settings};
use crate::i18n::Lang;
use crate::services::images::{add_image, MAX_UPLOAD_BYTES, UPLOAD_DIR};
use crate::services::videos;

use askama::Template;
use axum::extract::multipart::Field;
use axum::extract::{Extension, Multipart, Path, Query, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Redirect, Response};
use std::path::PathBuf;

/// The largest upload of several files together (two videos fit).
pub const MAX_UPLOAD_REQUEST_BYTES: usize = 10 * MAX_UPLOAD_BYTES;

/// Why an upload was refused (English; `lang.t` translates them).
const TOO_LARGE: &str = "larger than 20 MB: save it smaller and upload it again";
const UPLOAD_CUT_SHORT: &str = "the upload didn't finish: please try again";

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
    /// A video (MP4 or WebM) rather than a picture.
    pub is_video: bool,
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
        let is_video = m.filename.ends_with(".mp4") || m.filename.ends_with(".webm");
        Self {
            is_video,
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
    /// Choosing a video (the editor's Video button) rather than a picture.
    pub videos: bool,
}

/// `?videos=1`: the chooser shows videos instead of pictures.
#[derive(serde::Deserialize)]
pub struct PickerQuery {
    #[serde(default)]
    videos: Option<String>,
}

impl PickerQuery {
    fn videos(&self) -> bool {
        self.videos.is_some()
    }
}

/// The library's pictures, or its videos.
async fn media_of_kind(state: &AppState, videos: bool) -> Vec<MediaView> {
    all_media(state)
        .await
        .into_iter()
        .filter(|item| item.is_video == videos)
        .collect()
}

async fn all_media(state: &AppState) -> Vec<MediaView> {
    media::all(&state.pool)
        .await
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
    Query(query): Query<PickerQuery>,
) -> impl IntoResponse {
    MediaPickerTemplate {
        me,
        media: media_of_kind(&state, query.videos()).await,
        error: None,
        videos: query.videos(),
    }
}

/// POST /admin/media/picker -> Upload from the chooser, then show it again.
pub async fn picker_upload(
    State(state): State<AppState>,
    Extension(me): Extension<CurrentUser>,
    Query(query): Query<PickerQuery>,
    multipart: Multipart,
) -> impl IntoResponse {
    let errors = receive_uploads(&state, me.lang, multipart).await;
    MediaPickerTemplate {
        me,
        media: media_of_kind(&state, query.videos()).await,
        error: (!errors.is_empty()).then(|| errors.join(" · ")),
        videos: query.videos(),
    }
}

/// Process and save every file in the `files` field; returns one message per failure.
async fn receive_uploads(state: &AppState, lang: Lang, mut multipart: Multipart) -> Vec<String> {
    let mut errors = Vec::new();

    loop {
        let mut field = match multipart.next_field().await {
            Ok(Some(field)) => field,
            Ok(None) => break,
            Err(e) => {
                tracing::warn!("Upload stopped: {e}");
                let reason = if e.status() == StatusCode::PAYLOAD_TOO_LARGE {
                    "The files together are larger than 200 MB. Upload fewer at a time."
                } else {
                    "The upload didn't finish. Please try again."
                };
                errors.push(lang.t(reason).to_string());
                break;
            }
        };
        if field.name() != Some("files") {
            continue;
        }

        let original_name = field.file_name().unwrap_or("upload").to_string();
        // The first bytes say what the file is: a picture is read into memory
        // (it's resized there), a video goes straight to disk.
        let mut data = Vec::new();
        let mut refused = None;
        let mut finished = false;
        while data.len() < 64 && !finished {
            match field.chunk().await {
                Ok(Some(chunk)) => data.extend_from_slice(&chunk),
                Ok(None) => finished = true,
                Err(e) => {
                    tracing::warn!("Upload stopped: {e}");
                    refused = Some(UPLOAD_CUT_SHORT);
                    finished = true;
                }
            }
        }
        if refused.is_none() {
            if let Some(kind) = videos::kind_of(&data) {
                let saved = match receive_video(&mut field, data, finished).await {
                    Ok(temporary) => {
                        videos::add_video(state, &original_name, &temporary, kind).await
                    }
                    Err(reason) => Err(reason.to_string()),
                };
                if let Err(e) = saved {
                    errors.push(format!("{original_name}: {}", lang.t_owned(&e)));
                }
                continue;
            }
            if videos::looks_like_mp4_family(&data) {
                errors.push(format!(
                    "{original_name}: {}",
                    lang.t_owned(&videos::not_a_video())
                ));
                continue;
            }
        }
        // A picture: read the rest a piece at a time and stop past the limit,
        // so one huge file is refused without filling the memory, and the
        // others still go in. (The first piece can be big too.)
        if data.len() > MAX_UPLOAD_BYTES {
            refused = refused.or(Some(TOO_LARGE));
        }
        while refused.is_none() && !finished {
            match field.chunk().await {
                Ok(Some(chunk)) if data.len() + chunk.len() > MAX_UPLOAD_BYTES => {
                    refused = Some(TOO_LARGE);
                }
                Ok(Some(chunk)) => data.extend_from_slice(&chunk),
                Ok(None) => finished = true,
                Err(e) => {
                    tracing::warn!("Upload stopped: {e}");
                    refused = Some(UPLOAD_CUT_SHORT);
                }
            }
        }
        if let Some(reason) = refused {
            errors.push(format!("{original_name}: {}", lang.t(reason)));
            continue;
        }
        if data.is_empty() {
            continue;
        }

        if let Err(e) = add_image(state, &original_name, data).await {
            errors.push(format!("{original_name}: {}", lang.t_owned(&e)));
        }
    }

    errors
}

/// Save the rest of a video upload (after its first bytes, `start`) to a
/// temporary file in `uploads/`, a piece at a time. Gives back where it is,
/// or why it was refused; nothing is left behind then.
async fn receive_video(
    field: &mut Field<'_>,
    start: Vec<u8>,
    finished: bool,
) -> Result<PathBuf, &'static str> {
    use tokio::io::AsyncWriteExt;

    // A dot first and a random name: never listed, and nobody can guess it
    // while it's still arriving.
    let temporary =
        PathBuf::from(UPLOAD_DIR).join(format!(".upload-{}", crate::app::security::random_hex(12)));
    let result = async {
        tokio::fs::create_dir_all(UPLOAD_DIR)
            .await
            .map_err(|_| "could not save file")?;
        let mut file = tokio::fs::File::create(&temporary)
            .await
            .map_err(|_| "could not save file")?;
        file.write_all(&start)
            .await
            .map_err(|_| "could not save file")?;
        let mut size = start.len() as u64;
        if size > videos::MAX_VIDEO_BYTES {
            return Err(videos::VIDEO_TOO_LARGE);
        }
        // Nothing more to read when the whole file came in the first piece.
        let mut next = if finished {
            None
        } else {
            field.chunk().await.map_err(|_| UPLOAD_CUT_SHORT)?
        };
        while let Some(chunk) = next {
            size += chunk.len() as u64;
            if size > videos::MAX_VIDEO_BYTES {
                return Err(videos::VIDEO_TOO_LARGE);
            }
            file.write_all(&chunk)
                .await
                .map_err(|_| "could not save file")?;
            next = field.chunk().await.map_err(|_| UPLOAD_CUT_SHORT)?;
        }
        file.flush().await.map_err(|_| "could not save file")
    }
    .await;
    match result {
        Ok(()) => Ok(temporary),
        Err(reason) => {
            let _ = tokio::fs::remove_file(&temporary).await;
            Err(reason)
        }
    }
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
    match media::set_alt_text(&state.pool, id, &alt).await {
        Ok(true) => me.t("Saved").into_response(),
        Ok(false) => (StatusCode::NOT_FOUND, me.t("Image not found")).into_response(),
        Err(e) => {
            tracing::error!("Failed to save image description: {e}");
            let message = me.t("Couldn't save. Please try again.");
            (StatusCode::INTERNAL_SERVER_ERROR, message).into_response()
        }
    }
}

/// DELETE /admin/media/:id -> Remove a file from the library and disk.
pub async fn delete(State(state): State<AppState>, Path(id): Path<i64>) -> StatusCode {
    let Some(files) = media::delete(&state.pool, id).await else {
        return StatusCode::NOT_FOUND;
    };
    for name in files {
        let path = PathBuf::from(UPLOAD_DIR).join(&name);
        if let Err(e) = tokio::fs::remove_file(&path).await {
            tracing::error!("Failed to remove upload {name}: {e}");
        }
        // Its WebP copy, if it has one.
        let _ = tokio::fs::remove_file(path.with_file_name(format!("{name}.webp"))).await;
    }
    StatusCode::OK
}
