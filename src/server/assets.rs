//! The admin panel's CSS and JS, compiled into the binary so a deploy is a
//! single executable.

use axum::extract::Path;
use axum::http::{header, HeaderMap, StatusCode};
use axum::response::{IntoResponse, Response};
use rust_embed::RustEmbed;

/// Admin panel CSS and JS, served from memory at `/static`.
#[derive(RustEmbed)]
#[folder = "admin/static/"]
struct StaticAssets;

/// GET /static/*path -> Embedded admin asset, with ETag revalidation.
pub async fn serve_static(Path(path): Path<String>, headers: HeaderMap) -> Response {
    let Some(file) = StaticAssets::get(&path) else {
        return StatusCode::NOT_FOUND.into_response();
    };

    let etag = format!(
        "\"{}\"",
        file.metadata
            .sha256_hash()
            .iter()
            .take(8)
            .map(|b| format!("{b:02x}"))
            .collect::<String>()
    );
    let cache_headers = [
        (header::ETAG, etag.clone()),
        // File names don't change between versions, so browsers must check
        // for a newer copy each time (a quick 304 when nothing changed).
        (header::CACHE_CONTROL, "no-cache".to_string()),
    ];

    if headers
        .get(header::IF_NONE_MATCH)
        .is_some_and(|v| v.as_bytes() == etag.as_bytes())
    {
        return (StatusCode::NOT_MODIFIED, cache_headers).into_response();
    }

    (
        cache_headers,
        [(header::CONTENT_TYPE, file.metadata.mimetype().to_string())],
        file.data.into_owned(),
    )
        .into_response()
}
