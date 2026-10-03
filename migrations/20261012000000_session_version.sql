-- Every sign-in session remembers the account's session version. Raising the
-- number (e.g. after a password change) signs the account out everywhere.
ALTER TABLE users ADD COLUMN session_version INTEGER NOT NULL DEFAULT 0;
