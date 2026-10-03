-- Reader comments. Nothing identifying is stored beyond what the reader types;
-- the email is optional and never shown.
CREATE TABLE IF NOT EXISTS comments (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    post_id INTEGER NOT NULL,
    author_name TEXT NOT NULL,
    author_email TEXT NOT NULL DEFAULT '',
    content TEXT NOT NULL,
    -- pending, approved or spam
    status TEXT NOT NULL DEFAULT 'pending',
    created_at DATETIME NOT NULL DEFAULT CURRENT_TIMESTAMP,
    FOREIGN KEY (post_id) REFERENCES posts(id) ON DELETE CASCADE
);

CREATE INDEX IF NOT EXISTS idx_comments_post ON comments(post_id, status, id);
CREATE INDEX IF NOT EXISTS idx_comments_status ON comments(status, id DESC);

-- off, moderated (approve each one) or open
INSERT OR IGNORE INTO settings (key, value) VALUES ('comments', 'moderated');
