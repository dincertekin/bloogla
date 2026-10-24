//! The media library: the pictures people uploaded, and their smaller copies.
//! The files themselves are in `uploads/`; this is the list of them.

use crate::app::models::Media;
use crate::db::or_log;

use sqlx::SqlitePool;

/// A picture's file, its 800px and 400px copies, width and height.
pub type PictureCopies = (
    String,
    Option<String>,
    Option<String>,
    Option<i64>,
    Option<i64>,
);

/// A picture about to be added to the library.
pub struct NewMedia<'a> {
    pub filename: &'a str,
    pub small_filename: Option<&'a str>,
    pub thumb_filename: Option<&'a str>,
    pub original_name: &'a str,
    pub mime_type: &'a str,
    pub size_bytes: i64,
    pub width: i64,
    pub height: i64,
}

/// Every picture, newest first.
pub async fn all(pool: &SqlitePool) -> Vec<Media> {
    or_log(
        sqlx::query_as(
            "SELECT id, filename, small_filename, thumb_filename, original_name, alt_text,
                    size_bytes, width, height
             FROM media ORDER BY id DESC",
        )
        .fetch_all(pool)
        .await,
        "list media",
    )
}

/// The smaller copies of the pictures with these file names, read in one
/// query for a whole post.
pub async fn copies_of(pool: &SqlitePool, filenames: &[&str]) -> Vec<PictureCopies> {
    if filenames.is_empty() {
        return Vec::new();
    }
    let marks = vec!["?"; filenames.len()].join(", ");
    let sql = format!(
        "SELECT filename, small_filename, thumb_filename, width, height FROM media
         WHERE filename IN ({marks})"
    );
    let mut query = sqlx::query_as::<_, PictureCopies>(&sql);
    for name in filenames {
        query = query.bind(*name);
    }
    or_log(query.fetch_all(pool).await, "find picture copies")
}

/// Add a picture whose files are already saved.
pub async fn add(pool: &SqlitePool, media: &NewMedia<'_>) -> Result<(), sqlx::Error> {
    sqlx::query(
        "INSERT INTO media (filename, small_filename, thumb_filename, original_name, mime_type,
                            size_bytes, width, height)
         VALUES (?, ?, ?, ?, ?, ?, ?, ?)",
    )
    .bind(media.filename)
    .bind(media.small_filename)
    .bind(media.thumb_filename)
    .bind(media.original_name)
    .bind(media.mime_type)
    .bind(media.size_bytes)
    .bind(media.width)
    .bind(media.height)
    .execute(pool)
    .await?;
    Ok(())
}

/// Save what a picture shows. `false` when there is no such picture.
pub async fn set_alt_text(pool: &SqlitePool, id: i64, alt_text: &str) -> Result<bool, sqlx::Error> {
    let result = sqlx::query("UPDATE media SET alt_text = ? WHERE id = ?")
        .bind(alt_text)
        .bind(id)
        .execute(pool)
        .await?;
    Ok(result.rows_affected() > 0)
}

/// Take a picture out of the library and give back its file names (the
/// picture, then its copies) so they can be removed from `uploads/` too.
/// `None` when there is no such picture.
pub async fn delete(pool: &SqlitePool, id: i64) -> Option<Vec<String>> {
    let files: Option<(String, Option<String>, Option<String>)> = or_log(
        sqlx::query_as(
            "DELETE FROM media WHERE id = ? RETURNING filename, small_filename, thumb_filename",
        )
        .bind(id)
        .fetch_optional(pool)
        .await,
        "delete media",
    );
    let (filename, small, thumb) = files?;
    Some(
        std::iter::once(filename)
            .chain(small)
            .chain(thumb)
            .collect(),
    )
}

/// JPEG and PNG pictures wider than `width` that have no 400px copy yet:
/// `(id, file name)`.
pub async fn without_thumb(pool: &SqlitePool, width: u32) -> Vec<(i64, String)> {
    or_log(
        sqlx::query_as(
            "SELECT id, filename FROM media
             WHERE thumb_filename IS NULL AND width > ?
               AND mime_type IN ('image/jpeg', 'image/png')",
        )
        .bind(width)
        .fetch_all(pool)
        .await,
        "find images without a small copy",
    )
}

/// Record a picture's new 400px copy.
pub async fn set_thumb(
    pool: &SqlitePool,
    id: i64,
    thumb_filename: &str,
) -> Result<(), sqlx::Error> {
    sqlx::query("UPDATE media SET thumb_filename = ? WHERE id = ?")
        .bind(thumb_filename)
        .bind(id)
        .execute(pool)
        .await?;
    Ok(())
}
