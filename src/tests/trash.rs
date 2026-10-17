//! The trash, and changing many posts at once (Posts and Pages).

use super::{TestSite, ADMIN_EMAIL};
use axum::http::StatusCode;

async fn id_of(site: &TestSite, slug: &str) -> i64 {
    sqlx::query_scalar("SELECT id FROM posts WHERE slug = ?")
        .bind(slug)
        .fetch_one(&site.pool)
        .await
        .unwrap()
}

async fn count(site: &TestSite, sql: &str) -> i64 {
    sqlx::query_scalar(sql).fetch_one(&site.pool).await.unwrap()
}

#[tokio::test]
async fn a_trashed_post_leaves_the_site_and_can_come_back() {
    let mut site = TestSite::with_owner().await;
    let address = site.publish("Old opening hours", "published").await;
    let id = id_of(&site, "old-opening-hours").await;

    let trashed = site.post(&format!("/admin/posts/{id}/trash"), &[]).await;
    assert_eq!(trashed.status, StatusCode::OK);
    assert!(trashed
        .body
        .contains("“Old opening hours” moved to the trash."));
    assert!(trashed.body.contains("Undo"));

    // Gone from the site: its page, the home page, feeds and the sitemap.
    site.new_visitor();
    assert_eq!(site.get(&address).await.status, StatusCode::NOT_FOUND);
    assert!(!site.get("/").await.body.contains("Old opening hours"));
    assert!(!site
        .get("/rss.xml")
        .await
        .body
        .contains("old-opening-hours"));
    assert!(!site
        .get("/sitemap.xml")
        .await
        .body
        .contains("old-opening-hours"));
    site.sign_in(ADMIN_EMAIL, super::ADMIN_PASSWORD).await;

    // In the admin: only in the Trash, which now has a tab.
    let list = site.get("/admin/posts").await.body;
    assert!(!list.contains("Old opening hours"));
    assert!(list.contains("?status=trash"));
    let trash = site.get("/admin/posts?status=trash").await.body;
    assert!(trash.contains("Old opening hours"));
    assert!(trash.contains("Moved to the trash today"));
    // Not on the site, but not "Scheduled" either.
    assert!(!trash.contains("badge-scheduled"));
    assert!(trash.contains("deleted for good after 30 days"));

    // Undo puts it back as it was.
    let undo = site
        .post(&format!("/admin/posts/{id}/restore?undo=1"), &[])
        .await;
    assert_eq!(undo.header("hx-refresh"), "true");
    site.new_visitor();
    assert_eq!(site.get(&address).await.status, StatusCode::OK);
}

#[tokio::test]
async fn only_posts_in_the_trash_can_be_deleted_for_good() {
    let mut site = TestSite::with_owner().await;
    site.publish("Keep me", "published").await;
    let id = id_of(&site, "keep-me").await;
    let request = axum::http::Request::delete(format!("/admin/posts/{id}"))
        .header("sec-fetch-site", "same-origin");
    let refused = site.send(request, axum::body::Body::empty()).await;
    assert_eq!(refused.status, StatusCode::NOT_FOUND);
    assert_eq!(count(&site, "SELECT COUNT(*) FROM posts").await, 1);

    site.post(&format!("/admin/posts/{id}/trash"), &[]).await;
    let request = axum::http::Request::delete(format!("/admin/posts/{id}"))
        .header("sec-fetch-site", "same-origin");
    let deleted = site.send(request, axum::body::Body::empty()).await;
    assert_eq!(deleted.status, StatusCode::OK);
    assert_eq!(count(&site, "SELECT COUNT(*) FROM posts").await, 0);
}

#[tokio::test]
async fn emptying_the_trash_saves_a_backup_first() {
    let mut site = TestSite::with_owner().await;
    site.publish("Stays", "published").await;
    for title in ["Goes one", "Goes two"] {
        site.publish(title, "draft").await;
        let id = id_of(&site, &crate::content::text::slugify(title)).await;
        site.post(&format!("/admin/posts/{id}/trash"), &[]).await;
    }
    let trash = site.get("/admin/posts?status=trash").await.body;
    assert!(trash.contains("Empty trash"));
    assert!(trash.contains("The 2 posts in the trash will be deleted permanently"));

    let request =
        axum::http::Request::delete("/admin/posts/trash").header("sec-fetch-site", "same-origin");
    let emptied = site.send(request, axum::body::Body::empty()).await;
    assert_eq!(emptied.status, StatusCode::OK);
    assert_eq!(count(&site, "SELECT COUNT(*) FROM posts").await, 1);
    let backups = std::fs::read_dir(site.folder.join("data/backups")).unwrap();
    assert!(backups.flatten().any(|f| f
        .file_name()
        .to_string_lossy()
        .starts_with("before-empty-trash-")));
    assert!(site
        .get("/admin/posts?status=trash")
        .await
        .body
        .contains("The trash is empty."));
}

#[tokio::test]
async fn the_trash_empties_itself_after_30_days() {
    let site = TestSite::with_owner().await;
    sqlx::query(
        "INSERT INTO posts (title, slug, content, trashed_at) VALUES
         ('Long ago', 'long-ago', '', datetime('now', '-31 days')),
         ('Last week', 'last-week', '', datetime('now', '-7 days'))",
    )
    .execute(&site.pool)
    .await
    .unwrap();
    crate::db::posts::delete_old_trash(&site.pool).await;
    assert_eq!(
        count(&site, "SELECT COUNT(*) FROM posts WHERE slug = 'long-ago'").await,
        0
    );
    assert_eq!(
        count(&site, "SELECT COUNT(*) FROM posts WHERE slug = 'last-week'").await,
        1
    );
}

#[tokio::test]
async fn comments_on_trashed_posts_are_not_waiting_for_anyone() {
    let mut site = TestSite::with_owner().await;
    site.publish("Talk", "published").await;
    let id = id_of(&site, "talk").await;
    sqlx::query("INSERT INTO comments (post_id, author_name, content, status) VALUES (?, 'Reader', 'Hello', 'pending')")
        .bind(id)
        .execute(&site.pool)
        .await
        .unwrap();
    let before = site.get("/admin/comments").await.body;
    assert!(before.contains("Hello"));
    assert!(before.contains("1 comment waiting for approval"));
    site.post(&format!("/admin/posts/{id}/trash"), &[]).await;
    // Not listed, and not counted in the menu's badge either.
    let after = site.get("/admin/comments").await.body;
    assert!(!after.contains("Hello"));
    assert!(after.contains("0 comments waiting for approval"));
}

#[tokio::test]
async fn ticked_posts_change_together() {
    let mut site = TestSite::with_owner().await;
    for title in ["First draft", "Second draft", "Published one"] {
        let status = if title.ends_with("draft") {
            "draft"
        } else {
            "published"
        };
        site.publish(title, status).await;
    }
    // Drafts written long ago come out with today's date.
    sqlx::query("UPDATE posts SET published_at = '2020-01-01 10:00:00' WHERE status = 'draft'")
        .execute(&site.pool)
        .await
        .unwrap();
    let first = id_of(&site, "first-draft").await.to_string();
    let second = id_of(&site, "second-draft").await.to_string();
    let published = id_of(&site, "published-one").await.to_string();

    let list = site.get("/admin/posts").await.body;
    assert!(list.contains(r#"name="ids""#));
    assert!(list.contains("Select all on this page"));

    let done = site
        .post(
            "/admin/posts/bulk",
            &[("action", "publish"), ("ids", &first), ("ids", &second)],
        )
        .await;
    assert_eq!(done.header("hx-trigger"), "reload-page");
    assert_eq!(
        count(
            &site,
            "SELECT COUNT(*) FROM posts WHERE status = 'published' AND published_at > '2025-01-01'"
        )
        .await,
        3
    );

    site.post(
        "/admin/posts/bulk",
        &[("action", "draft"), ("ids", &published)],
    )
    .await;
    assert_eq!(
        count(&site, "SELECT COUNT(*) FROM posts WHERE status = 'draft'").await,
        1
    );

    site.post(
        "/admin/posts/bulk",
        &[
            ("action", "trash"),
            ("ids", &first),
            ("ids", &second),
            ("ids", &published),
        ],
    )
    .await;
    assert_eq!(
        count(
            &site,
            "SELECT COUNT(*) FROM posts WHERE trashed_at IS NOT NULL"
        )
        .await,
        3
    );
    let trash = site.get("/admin/posts?status=trash").await.body;
    assert!(trash.contains(r#"value="restore""#));

    site.post(
        "/admin/posts/bulk",
        &[("action", "restore"), ("ids", &first)],
    )
    .await;
    site.post(
        "/admin/posts/bulk",
        &[("action", "delete"), ("ids", &second), ("ids", &published)],
    )
    .await;
    assert_eq!(count(&site, "SELECT COUNT(*) FROM posts").await, 1);
    assert_eq!(
        count(&site, "SELECT COUNT(*) FROM posts WHERE trashed_at IS NULL").await,
        1
    );

    let nothing = site.post("/admin/posts/bulk", &[("action", "trash")]).await;
    assert!(nothing.body.contains("Tick the posts to change first."));
}

#[tokio::test]
async fn authors_change_only_their_own_posts() {
    let mut site = TestSite::with_owner().await;
    site.publish("The owner's post", "published").await;
    let owners = id_of(&site, "the-owner-s-post").await;
    let password = "Author-Pass-123!";
    let hash = crate::app::security::hash_password(password).unwrap();
    sqlx::query("INSERT INTO users (email, password_hash, name, role) VALUES ('author@example.com', ?, 'Author', 'author')")
        .bind(hash)
        .execute(&site.pool)
        .await
        .unwrap();
    site.new_visitor();
    site.sign_in("author@example.com", password).await;

    let refused = site
        .post(&format!("/admin/posts/{owners}/trash"), &[])
        .await;
    assert_eq!(refused.status, StatusCode::FORBIDDEN);
    let bulk = site
        .post(
            "/admin/posts/bulk",
            &[("action", "trash"), ("ids", &owners.to_string())],
        )
        .await;
    assert!(bulk.body.contains("can't be changed by you"));
    assert_eq!(
        count(
            &site,
            "SELECT COUNT(*) FROM posts WHERE trashed_at IS NOT NULL"
        )
        .await,
        0
    );
}
