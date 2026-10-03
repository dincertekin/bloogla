-- Several people can write on one site.
--   admin:  everything, including settings and users
--   editor: all posts, pages, media and tags
--   author: their own posts
ALTER TABLE users ADD COLUMN name TEXT NOT NULL DEFAULT '';
ALTER TABLE users ADD COLUMN role TEXT NOT NULL DEFAULT 'admin';

-- Who wrote each post. Existing posts belong to the first account.
ALTER TABLE posts ADD COLUMN author_id INTEGER REFERENCES users(id) ON DELETE SET NULL;
UPDATE posts SET author_id = (SELECT MIN(id) FROM users);

-- The first account's display name: the author name from settings, unless that was an email.
UPDATE users SET name = COALESCE(
    (SELECT value FROM settings WHERE key = 'publisher_name' AND value NOT LIKE '%@%'),
    ''
) WHERE id = (SELECT MIN(id) FROM users);

CREATE INDEX IF NOT EXISTS idx_posts_author ON posts(author_id);
