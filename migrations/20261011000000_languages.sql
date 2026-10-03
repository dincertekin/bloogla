-- Interface language: the site's default (public pages, emails) and each
-- person's own choice for the admin panel ('' means "use the site's").
INSERT OR IGNORE INTO settings (key, value) VALUES ('language', 'en');
ALTER TABLE users ADD COLUMN language TEXT NOT NULL DEFAULT '';
