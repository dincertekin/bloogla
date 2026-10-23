-- Messages visitors send with the contact form ([contact-form] in a page),
-- read in Admin → Messages. `read_at` is empty until someone opens Messages.
CREATE TABLE messages (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    post_id INTEGER REFERENCES posts(id) ON DELETE SET NULL,
    name TEXT NOT NULL,
    email TEXT NOT NULL,
    message TEXT NOT NULL,
    created_at DATETIME NOT NULL DEFAULT CURRENT_TIMESTAMP,
    read_at DATETIME
);
