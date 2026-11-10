-- Readers who get new posts by email (Settings → Subscribers; off at first).
CREATE TABLE subscribers (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    email TEXT NOT NULL UNIQUE COLLATE NOCASE,
    -- The secret in the confirm and unsubscribe links.
    token TEXT NOT NULL UNIQUE,
    -- NULL until the reader opens the link in the first email.
    confirmed_at DATETIME,
    created_at DATETIME NOT NULL DEFAULT CURRENT_TIMESTAMP
);

-- Whether a post goes to subscribers once it's public (the editor's
-- checkbox), and when it went, so it's never sent twice.
ALTER TABLE posts ADD COLUMN email_subscribers INTEGER NOT NULL DEFAULT 0;
ALTER TABLE posts ADD COLUMN emailed_at DATETIME;
