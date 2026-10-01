-- Content is called "posts" from now on.

-- The full-text index is tied to the old table name; rebuild it afterwards.
DROP TRIGGER IF EXISTS articles_ai;
DROP TRIGGER IF EXISTS articles_ad;
DROP TRIGGER IF EXISTS articles_au;
DROP TABLE IF EXISTS articles_fts;
DROP INDEX IF EXISTS idx_articles_status_published;
DROP INDEX IF EXISTS idx_articles_slug;

ALTER TABLE articles RENAME TO posts;
ALTER TABLE article_tags RENAME TO post_tags;
ALTER TABLE post_tags RENAME COLUMN article_id TO post_id;
ALTER TABLE slug_redirects RENAME COLUMN article_id TO post_id;

CREATE INDEX IF NOT EXISTS idx_posts_status_published
    ON posts(status, published_at DESC);

CREATE VIRTUAL TABLE IF NOT EXISTS posts_fts USING fts5(
    title,
    content,
    content='posts',
    content_rowid='id'
);
INSERT INTO posts_fts(posts_fts) VALUES ('rebuild');

CREATE TRIGGER IF NOT EXISTS posts_ai AFTER INSERT ON posts BEGIN
    INSERT INTO posts_fts(rowid, title, content) VALUES (new.id, new.title, new.content);
END;

CREATE TRIGGER IF NOT EXISTS posts_ad AFTER DELETE ON posts BEGIN
    INSERT INTO posts_fts(posts_fts, rowid, title, content) VALUES ('delete', old.id, old.title, old.content);
END;

CREATE TRIGGER IF NOT EXISTS posts_au AFTER UPDATE ON posts BEGIN
    INSERT INTO posts_fts(posts_fts, rowid, title, content) VALUES ('delete', old.id, old.title, old.content);
    INSERT INTO posts_fts(rowid, title, content) VALUES (new.id, new.title, new.content);
END;

-- Only the default theme ships now.
UPDATE settings SET value = 'default' WHERE key = 'active_theme' AND value = 'classic';

-- Public view counts are opt-in.
INSERT OR IGNORE INTO settings (key, value) VALUES ('show_views', 'false');
