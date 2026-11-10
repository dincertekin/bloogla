-- Each person's Dashboard: the boxes in their order, hidden ones marked
-- with "-" (like "stats,views,-referrers"). '' is the usual order.
ALTER TABLE users ADD COLUMN dashboard_layout TEXT NOT NULL DEFAULT '';
