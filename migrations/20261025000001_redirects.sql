-- Redirects the owner adds in Settings → Redirects: visitors who open an old
-- address (`/old-page`, kept decoded) are sent on to the new one.
CREATE TABLE redirects (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    from_path TEXT NOT NULL UNIQUE,
    to_address TEXT NOT NULL,
    created_at DATETIME NOT NULL DEFAULT CURRENT_TIMESTAMP
);
