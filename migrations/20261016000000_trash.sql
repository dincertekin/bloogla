-- The trash: when a post or page was moved there (empty when it isn't).
-- Trashed posts are hidden from the site and deleted after 30 days.
ALTER TABLE posts ADD COLUMN trashed_at DATETIME;
CREATE INDEX idx_posts_trashed ON posts(trashed_at);
