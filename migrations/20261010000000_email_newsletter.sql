-- Newsletter subscribers. A subscription only becomes active after the reader
-- confirms it from the email we send (double opt-in). The token is used for
-- the confirm and unsubscribe links.
CREATE TABLE IF NOT EXISTS subscribers (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    email TEXT NOT NULL UNIQUE COLLATE NOCASE,
    -- pending, active or unsubscribed
    status TEXT NOT NULL DEFAULT 'pending',
    token TEXT NOT NULL UNIQUE,
    created_at DATETIME NOT NULL DEFAULT CURRENT_TIMESTAMP,
    confirmed_at DATETIME
);

-- Posts already emailed to subscribers (each post goes out at most once).
CREATE TABLE IF NOT EXISTS newsletter_sends (
    post_id INTEGER PRIMARY KEY,
    started_at DATETIME NOT NULL DEFAULT CURRENT_TIMESTAMP,
    recipients INTEGER NOT NULL DEFAULT 0,
    delivered INTEGER NOT NULL DEFAULT 0,
    FOREIGN KEY (post_id) REFERENCES posts(id) ON DELETE CASCADE
);

INSERT OR IGNORE INTO settings (key, value) VALUES ('newsletter', 'false');
INSERT OR IGNORE INTO settings (key, value) VALUES ('notify_comments', 'true');
INSERT OR IGNORE INTO settings (key, value) VALUES ('smtp_port', '587');
INSERT OR IGNORE INTO settings (key, value) VALUES ('smtp_security', 'starttls');
