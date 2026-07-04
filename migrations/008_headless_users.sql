ALTER TABLE users ADD COLUMN is_headless INTEGER NOT NULL DEFAULT 0;

UPDATE users SET is_headless = 1 WHERE username LIKE 'headless_%';
