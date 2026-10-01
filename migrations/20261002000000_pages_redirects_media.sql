-- Standalone pages (About, Contact...): served at /:slug and kept out of listings and feeds
ALTER TABLE articles ADD COLUMN is_page INTEGER NOT NULL DEFAULT 0;

-- Old slugs keep working after an article's URL changes (301 to the current slug)
CREATE TABLE IF NOT EXISTS slug_redirects (
    old_slug TEXT PRIMARY KEY,
    article_id INTEGER NOT NULL,
    FOREIGN KEY (article_id) REFERENCES articles(id) ON DELETE CASCADE
);

-- Uploaded media library
CREATE TABLE IF NOT EXISTS media (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    filename TEXT NOT NULL UNIQUE,
    original_name TEXT NOT NULL,
    mime_type TEXT NOT NULL,
    size_bytes INTEGER NOT NULL,
    width INTEGER,
    height INTEGER,
    created_at DATETIME NOT NULL DEFAULT CURRENT_TIMESTAMP
);
