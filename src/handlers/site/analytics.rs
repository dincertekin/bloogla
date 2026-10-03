//! Privacy-friendly view counts: no cookies and no personal data. Bots, link
//! previews, prefetches and the site's own signed-in people aren't counted.

use crate::app::models::SESSION_USER_ID;
use crate::app::state::AppState;

use axum::http::{header, HeaderMap};
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

/// Count a view in the background so the page isn't delayed.
pub fn record_view(state: &AppState, post_id: i64, referrer: Option<String>) {
    let pool = state.pool.clone();
    tokio::spawn(async move {
        // One transaction: a single write to disk instead of three.
        let result = async {
            let mut tx = pool.begin().await?;
            sqlx::query("UPDATE posts SET views = views + 1 WHERE id = ?")
                .bind(post_id)
                .execute(&mut *tx)
                .await?;
            sqlx::query(
                "INSERT INTO daily_views (day, post_id, views) VALUES (date('now'), ?, 1)
                 ON CONFLICT(day, post_id) DO UPDATE SET views = views + 1",
            )
            .bind(post_id)
            .execute(&mut *tx)
            .await?;
            if let Some(host) = referrer {
                sqlx::query(
                    "INSERT INTO daily_referrers (day, host, views) VALUES (date('now'), ?, 1)
                     ON CONFLICT(day, host) DO UPDATE SET views = views + 1",
                )
                .bind(host)
                .execute(&mut *tx)
                .await?;
            }
            tx.commit().await
        }
        .await;
        if let Err(e) = result {
            tracing::warn!("Could not count a view of post {post_id}: {e}");
        }
    });
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
