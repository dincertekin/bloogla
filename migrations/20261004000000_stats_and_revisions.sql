-- Privacy-friendly analytics: daily totals only, no visitor data.
CREATE TABLE IF NOT EXISTS daily_views (
    day TEXT NOT NULL,
    post_id INTEGER NOT NULL,
    views INTEGER NOT NULL DEFAULT 0,
    PRIMARY KEY (day, post_id),
    FOREIGN KEY (post_id) REFERENCES posts(id) ON DELETE CASCADE
);

-- Sites that link to posts, by host name.
CREATE TABLE IF NOT EXISTS daily_referrers (
    day TEXT NOT NULL,
    host TEXT NOT NULL,
    views INTEGER NOT NULL DEFAULT 0,
    PRIMARY KEY (day, host)
);

-- Earlier versions of a post, saved each time it changes.
CREATE TABLE IF NOT EXISTS post_revisions (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    post_id INTEGER NOT NULL,
    title TEXT NOT NULL,
    content TEXT NOT NULL,
    saved_at DATETIME NOT NULL DEFAULT CURRENT_TIMESTAMP,
    FOREIGN KEY (post_id) REFERENCES posts(id) ON DELETE CASCADE
);

CREATE INDEX IF NOT EXISTS idx_post_revisions_post ON post_revisions(post_id, id DESC);
