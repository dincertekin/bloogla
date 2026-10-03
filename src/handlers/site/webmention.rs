//! Receiving webmentions: another site telling us it links to one of our posts.
//! The protocol itself is in `src/services/webmention.rs`.

use crate::app::state::AppState;

use axum::extract::{ConnectInfo, State};
use axum::http::{HeaderMap, StatusCode};
use axum::response::{IntoResponse, Response};
use axum_extra::extract::Form;
use reqwest::Url;
use serde::Deserialize;
use std::net::SocketAddr;

#[derive(Deserialize)]
pub struct MentionForm {
    source: String,
    target: String,
}

/// POST /webmention -> Accept a mention and check it in the background.
pub async fn receive(
    State(state): State<AppState>,
    ConnectInfo(peer): ConnectInfo<SocketAddr>,
    headers: HeaderMap,
    Form(form): Form<MentionForm>,
) -> Response {
    let (Ok(source), Ok(target)) = (
        Url::parse(form.source.trim()),
        Url::parse(form.target.trim()),
    ) else {
        return (StatusCode::BAD_REQUEST, "source and target must be URLs").into_response();
    };
    if source == target || !matches!(source.scheme(), "http" | "https") {
        return (StatusCode::BAD_REQUEST, "source must be another web page").into_response();
    }
    let Some(post_id) = crate::services::webmention::target_post(&state, &target).await else {
        return (StatusCode::BAD_REQUEST, "target isn't a post on this site").into_response();
    };
    // Each mention makes the server fetch a page, so senders are rate-limited.
    if !super::allow_from(&state, peer, &headers) {
        return (
            StatusCode::TOO_MANY_REQUESTS,
            "Too many mentions; try again later",
        )
            .into_response();
    }

    crate::services::webmention::check_in_background(&state, post_id, source, target);
    (StatusCode::ACCEPTED, "Thanks! The mention will be checked.").into_response()
}
