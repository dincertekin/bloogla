-- Webmentions arrive as comments that link back to the page that mentioned us.
ALTER TABLE comments ADD COLUMN source_url TEXT;
CREATE UNIQUE INDEX IF NOT EXISTS idx_comments_source ON comments(post_id, source_url)
    WHERE source_url IS NOT NULL;

-- Links we've already notified, so saving a post doesn't notify them again.
CREATE TABLE IF NOT EXISTS webmentions_sent (
    post_id INTEGER NOT NULL,
    target TEXT NOT NULL,
    sent_at DATETIME NOT NULL DEFAULT CURRENT_TIMESTAMP,
    result TEXT NOT NULL,
    PRIMARY KEY (post_id, target),
    FOREIGN KEY (post_id) REFERENCES posts(id) ON DELETE CASCADE
);

-- Tell sites we link to (on by default, like WordPress pingbacks).
INSERT OR IGNORE INTO settings (key, value) VALUES ('send_webmentions', 'true');
