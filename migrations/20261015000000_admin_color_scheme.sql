-- Each person's choice for the admin panel: '' follows their device,
-- 'light' or 'dark' always (the switch at the bottom of the sidebar).
ALTER TABLE users ADD COLUMN color_scheme TEXT NOT NULL DEFAULT '';
