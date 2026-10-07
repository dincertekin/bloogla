-- Bloogla's database. SQLite runs this once on a new database.
--
-- To change the schema later, add a NEW file to this folder (named with a
-- later date) instead of editing this one: databases that already ran it
-- won't run it again.

-- People who can sign in.
CREATE TABLE users (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    email TEXT NOT NULL UNIQUE,
    password_hash TEXT NOT NULL,
    name TEXT NOT NULL DEFAULT '',
    -- admin, editor or author
    role TEXT NOT NULL DEFAULT 'author',
    -- Admin panel language; '' means the site's language.
    language TEXT NOT NULL DEFAULT '',
    -- Raised to sign the account out everywhere (e.g. after a password change).
    session_version INTEGER NOT NULL DEFAULT 0,
    -- Two-factor login: the authenticator key (encrypted, see src/app/secrets.rs),
    -- NULL when it's off, and the last time step used, so a code works once.
    totp_secret TEXT,
    totp_last_step INTEGER NOT NULL DEFAULT 0,
    created_at DATETIME NOT NULL DEFAULT CURRENT_TIMESTAMP
);

-- One-time recovery codes for two-factor login (only hashes are stored).
CREATE TABLE recovery_codes (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    user_id INTEGER NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    code_hash TEXT NOT NULL,
    used_at DATETIME
);
CREATE INDEX idx_recovery_codes_user ON recovery_codes(user_id);

-- Posts and standalone pages (is_page = 1).
CREATE TABLE posts (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    title TEXT NOT NULL,
    slug TEXT NOT NULL UNIQUE,
    -- Markdown
    content TEXT NOT NULL,
    cover_image TEXT,
    views INTEGER NOT NULL DEFAULT 0,
    -- draft, published or scheduled
    status TEXT NOT NULL DEFAULT 'published',
    published_at DATETIME NOT NULL DEFAULT CURRENT_TIMESTAMP,
    is_page INTEGER NOT NULL DEFAULT 0,
    author_id INTEGER REFERENCES users(id) ON DELETE SET NULL,
    created_at DATETIME NOT NULL DEFAULT CURRENT_TIMESTAMP
);
CREATE INDEX idx_posts_status_published ON posts(status, published_at DESC);
CREATE INDEX idx_posts_author ON posts(author_id);

-- Full-text search over titles and content, kept in step by the triggers.
CREATE VIRTUAL TABLE posts_fts USING fts5(
    title,
    content,
    content='posts',
    content_rowid='id'
);
CREATE TRIGGER posts_ai AFTER INSERT ON posts BEGIN
    INSERT INTO posts_fts(rowid, title, content) VALUES (new.id, new.title, new.content);
END;
CREATE TRIGGER posts_ad AFTER DELETE ON posts BEGIN
    INSERT INTO posts_fts(posts_fts, rowid, title, content) VALUES ('delete', old.id, old.title, old.content);
END;
CREATE TRIGGER posts_au AFTER UPDATE ON posts BEGIN
    INSERT INTO posts_fts(posts_fts, rowid, title, content) VALUES ('delete', old.id, old.title, old.content);
    INSERT INTO posts_fts(rowid, title, content) VALUES (new.id, new.title, new.content);
END;

-- Earlier versions of posts, shown in the editor (the newest 25 are kept).
CREATE TABLE post_revisions (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    post_id INTEGER NOT NULL REFERENCES posts(id) ON DELETE CASCADE,
    title TEXT NOT NULL,
    content TEXT NOT NULL,
    saved_at DATETIME NOT NULL DEFAULT CURRENT_TIMESTAMP
);
CREATE INDEX idx_post_revisions_post ON post_revisions(post_id, id DESC);

-- Old addresses of posts whose slug changed, so links keep working.
CREATE TABLE slug_redirects (
    old_slug TEXT PRIMARY KEY,
    post_id INTEGER NOT NULL REFERENCES posts(id) ON DELETE CASCADE
);

-- Custom fields: extra named values on a post.
CREATE TABLE post_fields (
    post_id INTEGER NOT NULL REFERENCES posts(id) ON DELETE CASCADE,
    key TEXT NOT NULL,
    value TEXT NOT NULL,
    PRIMARY KEY (post_id, key)
);

CREATE TABLE tags (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    name TEXT NOT NULL UNIQUE,
    slug TEXT NOT NULL UNIQUE
);

CREATE TABLE post_tags (
    post_id INTEGER NOT NULL REFERENCES posts(id) ON DELETE CASCADE,
    tag_id INTEGER NOT NULL REFERENCES tags(id) ON DELETE CASCADE,
    PRIMARY KEY (post_id, tag_id)
);

-- Uploaded images (files are in uploads/).
CREATE TABLE media (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    filename TEXT NOT NULL UNIQUE,
    -- An 800px-wide copy of wide photos, for phones and post cards.
    small_filename TEXT,
    original_name TEXT NOT NULL,
    mime_type TEXT NOT NULL,
    size_bytes INTEGER NOT NULL,
    width INTEGER,
    height INTEGER,
    created_at DATETIME NOT NULL DEFAULT CURRENT_TIMESTAMP
);

-- Site settings as key/value pairs; defaults live in src/db/settings.rs.
CREATE TABLE settings (
    key TEXT PRIMARY KEY,
    value TEXT NOT NULL
);

-- Reader comments and received webmentions (source_url is set for those).
CREATE TABLE comments (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    post_id INTEGER NOT NULL REFERENCES posts(id) ON DELETE CASCADE,
    author_name TEXT NOT NULL,
    author_email TEXT NOT NULL DEFAULT '',
    content TEXT NOT NULL,
    -- pending, approved or spam
    status TEXT NOT NULL DEFAULT 'pending',
    source_url TEXT,
    created_at DATETIME NOT NULL DEFAULT CURRENT_TIMESTAMP
);
CREATE INDEX idx_comments_post ON comments(post_id, status, id);
CREATE INDEX idx_comments_status ON comments(status, id DESC);
CREATE UNIQUE INDEX idx_comments_source ON comments(post_id, source_url) WHERE source_url IS NOT NULL;

-- Webmentions this site sent, so each link is notified once.
CREATE TABLE webmentions_sent (
    post_id INTEGER NOT NULL REFERENCES posts(id) ON DELETE CASCADE,
    target TEXT NOT NULL,
    result TEXT NOT NULL,
    sent_at DATETIME NOT NULL DEFAULT CURRENT_TIMESTAMP,
    PRIMARY KEY (post_id, target)
);

-- Privacy-friendly analytics: counts per day, no personal data.
CREATE TABLE daily_views (
    day TEXT NOT NULL,
    post_id INTEGER NOT NULL REFERENCES posts(id) ON DELETE CASCADE,
    views INTEGER NOT NULL DEFAULT 0,
    PRIMARY KEY (day, post_id)
);
CREATE TABLE daily_referrers (
    day TEXT NOT NULL,
    host TEXT NOT NULL,
    views INTEGER NOT NULL DEFAULT 0,
    PRIMARY KEY (day, host)
);

-- Personal API tokens (only a SHA-256 hash of each token is stored).
CREATE TABLE api_tokens (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    user_id INTEGER NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    name TEXT NOT NULL,
    token_hash TEXT NOT NULL UNIQUE,
    created_at DATETIME NOT NULL DEFAULT CURRENT_TIMESTAMP,
    last_used_at DATETIME
);

-- Newsletter subscribers (double opt-in) and which posts were emailed.
CREATE TABLE subscribers (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    email TEXT NOT NULL UNIQUE COLLATE NOCASE,
    -- pending, active or unsubscribed
    status TEXT NOT NULL DEFAULT 'pending',
    token TEXT NOT NULL UNIQUE,
    created_at DATETIME NOT NULL DEFAULT CURRENT_TIMESTAMP,
    confirmed_at DATETIME
);
CREATE TABLE newsletter_sends (
    post_id INTEGER PRIMARY KEY REFERENCES posts(id) ON DELETE CASCADE,
    recipients INTEGER NOT NULL DEFAULT 0,
    delivered INTEGER NOT NULL DEFAULT 0,
    started_at DATETIME NOT NULL DEFAULT CURRENT_TIMESTAMP
);
