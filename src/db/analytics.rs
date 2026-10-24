//! View counts for the Dashboard: per post, per day and per referring site.
//! Counting itself happens in `handlers/site/analytics.rs`.

use crate::db::or_log;

use sqlx::SqlitePool;
use std::collections::HashMap;

/// A post in the "most viewed" chart.
#[derive(sqlx::FromRow)]
pub struct TopPost {
    pub id: i64,
    pub title: String,
    pub views: i64,
}

/// Published (or scheduled) posts, drafts, and the views of all posts
/// together. Pages and posts in the trash don't count.
pub async fn totals(pool: &SqlitePool) -> (i64, i64, i64) {
    or_log(
        sqlx::query_as(
            "SELECT COUNT(*) FILTER (WHERE status != 'draft'),
                    COUNT(*) FILTER (WHERE status = 'draft'),
                    COALESCE(SUM(views), 0)
             FROM posts WHERE is_page = 0 AND trashed_at IS NULL",
        )
        .fetch_one(pool)
        .await,
        "dashboard totals",
    )
}

/// The five most viewed posts.
pub async fn top_posts(pool: &SqlitePool) -> Vec<TopPost> {
    or_log(
        sqlx::query_as(
            "SELECT id, title, views FROM posts
             WHERE is_page = 0 AND trashed_at IS NULL AND views > 0
             ORDER BY views DESC LIMIT 5",
        )
        .fetch_all(pool)
        .await,
        "top posts",
    )
}

/// Views per day over the last 30 days, keyed by `YYYY-MM-DD`. Days without
/// views are missing.
pub async fn daily_views(pool: &SqlitePool) -> HashMap<String, i64> {
    or_log(
        sqlx::query_as(
            "SELECT day, SUM(views) FROM daily_views
             WHERE day >= date('now', '-29 days') GROUP BY day",
        )
        .fetch_all(pool)
        .await,
        "daily views",
    )
    .into_iter()
    .collect()
}

/// The five sites that sent the most visitors in the last 30 days.
pub async fn top_referrers(pool: &SqlitePool) -> Vec<(String, i64)> {
    or_log(
        sqlx::query_as(
            "SELECT host, SUM(views) AS total FROM daily_referrers
             WHERE day >= date('now', '-29 days')
             GROUP BY host ORDER BY total DESC LIMIT 5",
        )
        .fetch_all(pool)
        .await,
        "top referrers",
    )
}

/// Add counted views: `posts` maps a post id to its new views, `referrers`
/// a referring site to the visitors it sent. All or nothing.
pub async fn add_views(
    pool: &SqlitePool,
    posts: &HashMap<i64, i64>,
    referrers: &HashMap<String, i64>,
) -> Result<(), sqlx::Error> {
    // IMMEDIATE takes the write lock first, so this waits for other writers
    // (busy_timeout) instead of failing halfway.
    let mut tx = pool.begin_with("BEGIN IMMEDIATE").await?;
    for (post_id, views) in posts {
        sqlx::query("UPDATE posts SET views = views + ? WHERE id = ?")
            .bind(views)
            .bind(post_id)
            .execute(&mut *tx)
            .await?;
        sqlx::query(
            "INSERT INTO daily_views (day, post_id, views)
             SELECT date('now'), id, ? FROM posts WHERE id = ?
             ON CONFLICT(day, post_id) DO UPDATE SET views = views + excluded.views",
        )
        .bind(views)
        .bind(post_id)
        .execute(&mut *tx)
        .await?;
    }
    for (host, views) in referrers {
        sqlx::query(
            "INSERT INTO daily_referrers (day, host, views) VALUES (date('now'), ?, ?)
             ON CONFLICT(day, host) DO UPDATE SET views = views + excluded.views",
        )
        .bind(host)
        .bind(views)
        .execute(&mut *tx)
        .await?;
    }
    tx.commit().await
}
