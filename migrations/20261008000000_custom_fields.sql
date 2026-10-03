-- Extra named values per post (e.g. location, rating), for themes and the API.
CREATE TABLE IF NOT EXISTS post_fields (
    post_id INTEGER NOT NULL,
    key TEXT NOT NULL,
    value TEXT NOT NULL,
    PRIMARY KEY (post_id, key),
    FOREIGN KEY (post_id) REFERENCES posts(id) ON DELETE CASCADE
);
