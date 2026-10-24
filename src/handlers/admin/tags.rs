//! Tags: listing, adding and removing them.

use crate::app::models::{CurrentUser, Tag};
use crate::app::state::AppState;
use crate::db::{settings, tags};
use crate::i18n::Lang;

use askama::Template;
use axum::extract::{Extension, Path, State};
use axum::http::StatusCode;
use axum::response::{Html, IntoResponse, Response};
use axum_extra::extract::Form;
use serde::Deserialize;

/// A tag with the number of posts using it.
pub struct TagRow {
    pub tag: Tag,
    pub post_count: i64,
}

/// Admin tags page template.
#[derive(Template)]
#[template(path = "tags.html")]
pub struct TagsTemplate {
    pub blog_name: String,
    pub me: CurrentUser,
    pub tags: Vec<TagRow>,
}

/// New tag row partial template.
#[derive(Template)]
#[template(path = "tag_item.html")]
pub struct TagItemTemplate {
    pub me: CurrentUser,
    pub row: TagRow,
}

#[derive(Deserialize)]
pub struct TagForm {
    name: String,
}

/// GET /admin/tags -> Show tags list.
pub async fn page(
    State(state): State<AppState>,
    Extension(me): Extension<CurrentUser>,
) -> impl IntoResponse {
    let rows = tags::with_post_counts(&state.pool).await;
    let tags = rows
        .into_iter()
        .map(|(tag, post_count)| TagRow { tag, post_count })
        .collect();
    let blog_name = settings::load(&state.pool).await.blog_name.clone();

    TagsTemplate {
        blog_name,
        me,
        tags,
    }
}

/// Show a message in the tag form instead of adding a row to the list.
fn tag_form_error(lang: Lang, message: &str) -> Response {
    (
        [
            ("HX-Retarget", "#tag-form-alert"),
            ("HX-Reswap", "innerHTML"),
        ],
        Html(format!(
            r#"<div class="alert alert-error">{}</div>"#,
            lang.t_owned(message)
        )),
    )
        .into_response()
}

/// POST /admin/tags/new -> Create new tag.
pub async fn create(
    State(state): State<AppState>,
    Extension(me): Extension<CurrentUser>,
    Form(form): Form<TagForm>,
) -> Response {
    let name = form.name.trim().to_string();
    if name.is_empty() {
        return tag_form_error(me.lang, "Enter a name for the tag.");
    }

    let slug = tags::slugify(&name);

    let new_id = match tags::create(&state.pool, &name, &slug).await {
        Ok(None) => {
            return tag_form_error(me.lang, "A tag with this name already exists.");
        }
        Ok(Some(id)) => id,
        Err(e) => {
            tracing::error!("Failed to insert tag: {e}");
            return tag_form_error(me.lang, "Could not save the tag. Please try again.");
        }
    };

    TagItemTemplate {
        me,
        row: TagRow {
            tag: Tag {
                id: new_id,
                name,
                slug,
            },
            post_count: 0,
        },
    }
    .into_response()
}

/// DELETE /admin/tags/:id -> Remove tag.
pub async fn delete(State(state): State<AppState>, Path(id): Path<i64>) -> impl IntoResponse {
    match tags::delete(&state.pool, id).await {
        Ok(true) => StatusCode::OK,
        Ok(false) => StatusCode::NOT_FOUND,
        Err(_) => StatusCode::INTERNAL_SERVER_ERROR,
    }
}
