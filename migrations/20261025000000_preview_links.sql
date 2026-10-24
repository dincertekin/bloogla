-- A private link to read a draft before it's published (/preview/<token>).
-- Empty when the post isn't shared.
ALTER TABLE posts ADD COLUMN preview_token TEXT;
CREATE UNIQUE INDEX idx_posts_preview_token ON posts(preview_token);
