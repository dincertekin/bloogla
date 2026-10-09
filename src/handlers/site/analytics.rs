//! Privacy-friendly view counts: no cookies and no personal data. Bots, link
//! previews, prefetches and the site's own signed-in people aren't counted.

use crate::app::models::SESSION_USER_ID;

use axum::http::{header, HeaderMap};
use sqlx::SqlitePool;
use std::collections::HashMap;
use std::sync::Mutex;
use std::time::Duration;
use tower_sessions::Session;

/// Whether a request is a person reading the page, not a bot, a link
/// preview, a browser prefetch, or the site's own admin.
pub async fn is_countable_view(headers: &HeaderMap, session: &Session) -> bool {
    const BOT_MARKERS: &[&str] = &[
        "bot",
        "crawl",
        "spider",
        "slurp",
        "preview",
        "fetch",
        "curl",
        "wget",
        "python",
        "http",
        "headless",
        "monitor",
        "scan",
        "facebookexternalhit",
        "embedly",
    ];

    let user_agent = headers
        .get(header::USER_AGENT)
        .and_then(|v| v.to_str().ok())
        .unwrap_or_default()
        .to_ascii_lowercase();
    if user_agent.is_empty() || BOT_MARKERS.iter().any(|m| user_agent.contains(m)) {
        return false;
    }

    let prefetch = ["purpose", "sec-purpose"].iter().any(|name| {
        headers
            .get(*name)
            .and_then(|v| v.to_str().ok())
            .is_some_and(|v| v.contains("prefetch") || v.contains("prerender"))
    });
    if prefetch {
        return false;
    }

    let signed_in: Option<i64> = session.get(SESSION_USER_ID).await.unwrap_or(None);
    signed_in.is_none()
}

/// Host of an external referring site (`news.ycombinator.com`), if any.
pub fn referrer_host(headers: &HeaderMap, base_url: &str) -> Option<String> {
    let referer = headers.get(header::REFERER)?.to_str().ok()?;
    let host = referer
        .split_once("://")?
        .1
        .split(['/', '?', '#', ':'])
        .next()?
        .trim_start_matches("www.")
        .to_ascii_lowercase();
    let own_host = base_url
        .split_once("://")
        .map_or(base_url, |(_, rest)| rest)
        .split([':', '/'])
        .next()
        .unwrap_or_default()
        .trim_start_matches("www.");
    (!host.is_empty() && host != own_host && host.len() <= 253).then_some(host)
}

/// Views counted since they were last saved: per post, and per referring
/// site. Saved together every few seconds by [`spawn_view_saver`], so a busy
/// site does one small write instead of one per visitor (and visitors never
/// wait for the database).
#[derive(Default)]
struct PendingViews {
    posts: HashMap<i64, i64>,
    referrers: HashMap<String, i64>,
}

static PENDING: Mutex<Option<PendingViews>> = Mutex::new(None);

/// How often counted views are written to the database.
const SAVE_EVERY: Duration = Duration::from_secs(5);

/// Count a view. It's saved with the others within a few seconds.
pub fn record_view(post_id: i64, referrer: Option<String>) {
    let Ok(mut guard) = PENDING.lock() else {
        return;
    };
    let pending = guard.get_or_insert_with(PendingViews::default);
    *pending.posts.entry(post_id).or_default() += 1;
    if let Some(host) = referrer {
        *pending.referrers.entry(host).or_default() += 1;
    }
}

/// Save counted views every few seconds while the server runs.
pub fn spawn_view_saver(pool: SqlitePool) {
    tokio::spawn(async move {
        let mut interval = tokio::time::interval(SAVE_EVERY);
        loop {
            interval.tick().await;
            save_views(&pool).await;
        }
    });
}

/// Write the counted views to the database now (also called when the server
/// stops, so none are lost). If that fails they're kept for the next try.
pub async fn save_views(pool: &SqlitePool) {
    let Some(pending) = PENDING.lock().ok().and_then(|mut guard| guard.take()) else {
        return;
    };
    if pending.posts.is_empty() && pending.referrers.is_empty() {
        return;
    }
    if let Err(e) = write_views(pool, &pending).await {
        tracing::warn!("Could not save view counts (trying again shortly): {e}");
        // Put them back, added to anything counted meanwhile.
        if let Ok(mut guard) = PENDING.lock() {
            let current = guard.get_or_insert_with(PendingViews::default);
            for (id, n) in pending.posts {
                *current.posts.entry(id).or_default() += n;
            }
            for (host, n) in pending.referrers {
                *current.referrers.entry(host).or_default() += n;
            }
        }
    }
}

async fn write_views(pool: &SqlitePool, pending: &PendingViews) -> Result<(), sqlx::Error> {
    // IMMEDIATE takes the write lock first, so this waits for other writers
    // (busy_timeout) instead of failing halfway.
    let mut tx = pool.begin_with("BEGIN IMMEDIATE").await?;
    for (post_id, views) in &pending.posts {
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
    for (host, views) in &pending.referrers {
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

#[cfg(test)]
mod tests {
    use super::referrer_host;
    use axum::http::{header, HeaderMap, HeaderValue};

    fn with_referer(value: &str) -> HeaderMap {
        let mut headers = HeaderMap::new();
        headers.insert(header::REFERER, HeaderValue::from_str(value).unwrap());
        headers
    }

    #[test]
    fn referrer_host_ignores_own_site() {
        let base = "https://dincertekin.com";
        assert_eq!(
            referrer_host(
                &with_referer("https://news.ycombinator.com/item?id=1"),
                base
            ),
            Some("news.ycombinator.com".into())
        );
        assert_eq!(
            referrer_host(&with_referer("https://www.google.com/"), base),
            Some("google.com".into())
        );
        assert_eq!(
            referrer_host(&with_referer("https://www.dincertekin.com/post/a"), base),
            None
        );
        assert_eq!(referrer_host(&HeaderMap::new(), base), None);
    }
}
