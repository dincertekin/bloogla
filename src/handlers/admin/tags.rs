//! Tags: listing, adding and removing them.

use crate::app::models::{CurrentUser, Tag};
use crate::app::state::AppState;
use crate::db::{or_log, settings, tags};
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
    let rows: Vec<(i64, String, String, i64)> = or_log(
        sqlx::query_as(
            "SELECT t.id, t.name, t.slug, COUNT(pt.post_id)
             FROM tags t LEFT JOIN post_tags pt ON pt.tag_id = t.id
             GROUP BY t.id ORDER BY t.name ASC",
        )
        .fetch_all(&state.pool)
        .await,
        "list tags",
    );
    let tags = rows
        .into_iter()
        .map(|(id, name, slug, post_count)| TagRow {
            tag: Tag { id, name, slug },
            post_count,
        })
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

    let result = sqlx::query("INSERT OR IGNORE INTO tags (name, slug) VALUES (?, ?)")
        .bind(&name)
        .bind(&slug)
        .execute(&state.pool)
        .await;

    let new_id = match result {
        Ok(r) if r.rows_affected() == 0 => {
            return tag_form_error(me.lang, "A tag with this name already exists.");
        }
        Ok(r) => r.last_insert_rowid(),
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
    let result = sqlx::query("DELETE FROM tags WHERE id = ?")
        .bind(id)
        .execute(&state.pool)
        .await;

    match result {
        Ok(r) if r.rows_affected() > 0 => StatusCode::OK,
        Ok(_) => StatusCode::NOT_FOUND,
        Err(_) => StatusCode::INTERNAL_SERVER_ERROR,
    }
}
