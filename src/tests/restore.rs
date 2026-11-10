//! Restoring a backup .zip in Settings → Backup: checking it, confirming,
//! and the restart that puts it in place.

use super::TestSite;
use crate::services::restore;
use axum::http::StatusCode;
use std::io::{Read, Write};
use std::path::Path;

/// A .zip holding these `(name, contents)` files.
fn zip_of(files: &[(&str, &[u8])]) -> Vec<u8> {
    let mut zip = zip::ZipWriter::new(std::io::Cursor::new(Vec::new()));
    for (name, contents) in files {
        zip.start_file(*name, zip::write::SimpleFileOptions::default())
            .unwrap();
        zip.write_all(contents).unwrap();
    }
    zip.finish().unwrap().into_inner()
}

/// The database file inside a downloaded backup.
fn database_in(backup: &[u8]) -> Vec<u8> {
    let mut archive = zip::ZipArchive::new(std::io::Cursor::new(backup)).unwrap();
    let mut database = Vec::new();
    archive
        .by_name("data/bloogla.db")
        .unwrap()
        .read_to_end(&mut database)
        .unwrap();
    database
}

/// The backup's database after running `sql` on it, in a new .zip.
async fn changed_backup(site: &TestSite, backup: &[u8], sql: &str) -> Vec<u8> {
    let path = site.folder.join("changed.db");
    std::fs::write(&path, database_in(backup)).unwrap();
    let pool = sqlx::SqlitePool::connect(&format!("sqlite://{}", path.display()))
        .await
        .unwrap();
    sqlx::query(sql).execute(&pool).await.unwrap();
    pool.close().await;
    let database = std::fs::read(&path).unwrap();
    zip_of(&[("data/bloogla.db", &database)])
}

async fn write_post(site: &mut TestSite, title: &str) {
    let page = site
        .post(
            "/admin/posts",
            &[
                ("title", title),
                ("content", "Text"),
                ("status", "published"),
            ],
        )
        .await;
    assert_eq!(page.status, StatusCode::SEE_OTHER);
}

/// Titles of the posts in the database at `path`.
async fn titles_in(path: &Path) -> Vec<String> {
    let pool = sqlx::SqlitePool::connect(&format!("sqlite://{}", path.display()))
        .await
        .unwrap();
    let titles = sqlx::query_scalar("SELECT title FROM posts ORDER BY title")
        .fetch_all(&pool)
        .await
        .unwrap();
    pool.close().await;
    titles
}

#[tokio::test]
async fn a_backup_is_checked_then_restored_on_the_next_start() {
    let mut site = TestSite::with_owner().await;
    write_post(&mut site, "Written before the backup").await;
    let uploads = site.folder.join("uploads");
    std::fs::create_dir_all(&uploads).unwrap();
    std::fs::write(uploads.join("old-photo.jpg"), b"old photo").unwrap();
    let backup = site.get("/admin/backup.zip").await.bytes;

    // Things change after the backup was made.
    write_post(&mut site, "Written after the backup").await;
    std::fs::remove_file(uploads.join("old-photo.jpg")).unwrap();
    std::fs::write(uploads.join("new-photo.jpg"), b"new photo").unwrap();

    // Checking shows what's inside and changes nothing.
    let checked = site
        .upload(
            "/admin/backup/restore/check",
            "backup",
            "backup.zip",
            &backup,
        )
        .await;
    assert!(checked.body.contains("Written before the backup"));
    assert!(checked.body.contains("1 post"));
    assert!(checked.body.contains("1 person"));
    assert!(checked.body.contains("Restore this backup"));
    assert!(uploads.join("new-photo.jpg").exists());

    // Confirming saves the site as it is now, then asks for a restart.
    let restored = site.post("/admin/backup/restore", &[]).await;
    assert!(
        restored.body.contains("Restoring the backup"),
        "{}",
        restored.body
    );
    assert_eq!(restored.header("hx-trigger"), "restarting");
    let saved_first: Vec<_> = std::fs::read_dir(site.folder.join("data/backups"))
        .unwrap()
        .flatten()
        .filter(|f| {
            f.file_name()
                .to_string_lossy()
                .starts_with("before-restore-")
        })
        .collect();
    assert_eq!(saved_first.len(), 1, "the site is saved before restoring");

    // On the next start, the backup takes the site's place.
    restore::finish_pending(&site.folder).unwrap();
    assert_eq!(
        titles_in(&site.folder.join("data/bloogla.db")).await,
        ["Written before the backup"]
    );
    assert!(uploads.join("old-photo.jpg").exists());
    assert!(!uploads.join("new-photo.jpg").exists());
    assert!(!site.folder.join("data/restore").exists());
}

#[tokio::test]
async fn a_checked_backup_that_isnt_confirmed_changes_nothing() {
    let mut site = TestSite::with_owner().await;
    let backup = site.get("/admin/backup.zip").await.bytes;
    site.upload(
        "/admin/backup/restore/check",
        "backup",
        "backup.zip",
        &backup,
    )
    .await;

    // Cancel forgets it.
    let cancelled = site.post("/admin/backup/restore/cancel", &[]).await;
    assert!(cancelled.body.contains("Check backup"));
    assert!(!site.folder.join("data/restore").exists());
    let page = site.post("/admin/backup/restore", &[]).await;
    assert!(page.body.contains("Check the backup again first."));

    // Never confirmed: thrown away on the next start.
    site.upload(
        "/admin/backup/restore/check",
        "backup",
        "backup.zip",
        &backup,
    )
    .await;
    restore::finish_pending(&site.folder).unwrap();
    assert!(!site.folder.join("data/restore").exists());
    assert!(!site.folder.join("data/bloogla.db").exists());
}

#[tokio::test]
async fn files_that_arent_backups_are_refused() {
    let mut site = TestSite::with_owner().await;
    let refused = |body: &str, message: &str| {
        assert!(body.contains(message), "expected {message:?} in {body}");
        assert!(!body.contains("Restore this backup"));
    };

    let page = site
        .upload(
            "/admin/backup/restore/check",
            "backup",
            "photo.jpg",
            b"not a zip",
        )
        .await;
    refused(&page.body, "isn&#x27;t a Bloogla backup");

    let no_database = zip_of(&[("uploads/photo.jpg", b"photo")]);
    let page = site
        .upload(
            "/admin/backup/restore/check",
            "backup",
            "b.zip",
            &no_database,
        )
        .await;
    refused(&page.body, "isn&#x27;t a Bloogla backup");

    let not_sqlite = zip_of(&[("data/bloogla.db", b"hello")]);
    let page = site
        .upload(
            "/admin/backup/restore/check",
            "backup",
            "b.zip",
            &not_sqlite,
        )
        .await;
    refused(&page.body, "isn&#x27;t a Bloogla backup");
    assert!(!site.folder.join("data/restore").exists());
}

#[tokio::test]
async fn names_inside_a_backup_cant_leave_its_folder() {
    let mut site = TestSite::with_owner().await;
    let backup = site.get("/admin/backup.zip").await.bytes;
    let database = database_in(&backup);
    let sneaky = zip_of(&[
        ("data/bloogla.db", &database),
        ("../escaped.txt", b"x"),
        ("uploads/../../escaped.txt", b"x"),
        ("data/secret.key", b"x"),
        ("uploads/.htaccess", b"x"),
        ("uploads/photo.jpg", b"photo"),
    ]);
    let page = site
        .upload("/admin/backup/restore/check", "backup", "b.zip", &sneaky)
        .await;
    assert!(page.body.contains("Restore this backup"), "{}", page.body);

    let staging = site.folder.join("data/restore");
    assert!(!site.folder.join("escaped.txt").exists());
    assert!(!site.folder.join("data/escaped.txt").exists());
    assert!(!staging.join("secret.key").exists());
    let images: Vec<_> = std::fs::read_dir(staging.join("uploads"))
        .unwrap()
        .flatten()
        .map(|f| f.file_name().to_string_lossy().into_owned())
        .collect();
    assert_eq!(images, ["photo.jpg"]);
}

#[tokio::test]
async fn backups_that_cant_be_used_say_why() {
    let mut site = TestSite::with_owner().await;
    let backup = site.get("/admin/backup.zip").await.bytes;

    let newer = changed_backup(
        &site,
        &backup,
        "INSERT INTO _sqlx_migrations (version, description, success, checksum, execution_time)
         VALUES (99991231000000, 'from the future', 1, x'00', 0)",
    )
    .await;
    let page = site
        .upload("/admin/backup/restore/check", "backup", "b.zip", &newer)
        .await;
    assert!(page.body.contains("made by a newer version of Bloogla"));

    let no_admin = changed_backup(&site, &backup, "UPDATE users SET role = 'author'").await;
    let page = site
        .upload("/admin/backup/restore/check", "backup", "b.zip", &no_admin)
        .await;
    assert!(page.body.contains("no admin account"));
}

#[tokio::test]
async fn backups_from_bloogla_1_0_0_still_restore() {
    // 1.0.0 put the database at the top of the .zip.
    let mut site = TestSite::with_owner().await;
    let backup = site.get("/admin/backup.zip").await.bytes;
    let old_layout = zip_of(&[("bloogla.db", &database_in(&backup))]);
    let page = site
        .upload(
            "/admin/backup/restore/check",
            "backup",
            "b.zip",
            &old_layout,
        )
        .await;
    assert!(page.body.contains("Restore this backup"), "{}", page.body);
}

#[tokio::test]
async fn signed_out_visitors_cant_restore() {
    let mut site = TestSite::with_owner().await;
    let backup = site.get("/admin/backup.zip").await.bytes;
    site.new_visitor();
    let page = site
        .upload("/admin/backup/restore/check", "backup", "b.zip", &backup)
        .await;
    assert_eq!(page.status, StatusCode::UNAUTHORIZED);
    assert!(!site.folder.join("data/restore").exists());
    let page = site.post("/admin/backup/restore", &[]).await;
    assert_eq!(page.status, StatusCode::UNAUTHORIZED);
}

/// A backup made by Bloogla 1.0 still restores: the database is upgraded
/// on the next start, and its posts, people and pictures read as before.
#[tokio::test]
async fn a_backup_from_version_1_0_still_restores() {
    let mut site = TestSite::with_owner().await;

    // A database exactly as 1.0 left it: only the first migration applied.
    // (Migration files are never edited, so this one is 1.0's.)
    let old = site.folder.join("old");
    std::fs::create_dir_all(old.join("migrations")).unwrap();
    std::fs::write(
        old.join("migrations/20261009000000_schema.sql"),
        include_str!("../../migrations/20261009000000_schema.sql"),
    )
    .unwrap();
    let old_database = old.join("bloogla.db");
    let pool = sqlx::SqlitePool::connect(&format!("sqlite://{}?mode=rwc", old_database.display()))
        .await
        .unwrap();
    sqlx::migrate::Migrator::new(old.join("migrations"))
        .await
        .unwrap()
        .run(&pool)
        .await
        .unwrap();
    for sql in [
        "INSERT INTO users (email, password_hash, name, role) VALUES ('old@example.com', 'x', 'Old Owner', 'admin')",
        "INSERT INTO media (filename, small_filename, original_name, mime_type, size_bytes, width, height)
         VALUES ('photo.jpg', NULL, 'Photo.jpg', 'image/jpeg', 9, 1600, 900)",
        "INSERT INTO posts (title, slug, content, cover_image, status, author_id)
         VALUES ('From version one', 'from-version-one', 'Old **text**', '/uploads/photo.jpg', 'published', 1)",
        "INSERT INTO tags (name, slug) VALUES ('Archive', 'archive-tag')",
        "INSERT INTO post_tags (post_id, tag_id) VALUES (1, 1)",
        "INSERT INTO settings (key, value) VALUES ('blog_name', 'Old Blog')",
    ] {
        sqlx::query(sql).execute(&pool).await.unwrap();
    }
    pool.close().await;
    let backup = zip_of(&[
        ("data/bloogla.db", &std::fs::read(&old_database).unwrap()),
        ("uploads/photo.jpg", b"old photo"),
    ]);

    let checked = site
        .upload(
            "/admin/backup/restore/check",
            "backup",
            "backup.zip",
            &backup,
        )
        .await;
    assert!(
        checked.body.contains("From version one"),
        "{}",
        checked.body
    );
    site.post("/admin/backup/restore", &[]).await;
    restore::finish_pending(&site.folder).unwrap();

    // The next start upgrades the database; everything reads.
    let restored = crate::db::connect(&format!(
        "sqlite://{}",
        site.folder.join("data/bloogla.db").display()
    ))
    .await
    .expect("the old database upgrades");
    let post = crate::db::posts::find(&restored, 1)
        .await
        .expect("the post");
    assert_eq!(post.title, "From version one");
    assert_eq!(post.cover_width, Some(1600));
    assert!(post.trashed_at.is_none());
    let tags = crate::db::tags::for_posts(&restored, &[1]).await;
    assert_eq!(tags[&1][0].name, "Archive");
    let owner: String = sqlx::query_scalar("SELECT name FROM users WHERE id = 1")
        .fetch_one(&restored)
        .await
        .unwrap();
    assert_eq!(owner, "Old Owner");
    assert!(site.folder.join("uploads/photo.jpg").exists());
    restored.close().await;
}
