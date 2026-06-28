-- Fix invite_codes foreign keys to SET NULL on user deletion
-- SQLite doesn't support ALTER COLUMN, so we recreate the table

CREATE TABLE invite_codes_new (
    id         INTEGER PRIMARY KEY AUTOINCREMENT,
    code       TEXT NOT NULL UNIQUE,
    created_by INTEGER REFERENCES users(id) ON DELETE SET NULL,
    used_by    INTEGER REFERENCES users(id) ON DELETE SET NULL,
    created_at TEXT NOT NULL DEFAULT (datetime('now')),
    used_at    TEXT,
    expires_at TEXT
);

INSERT INTO invite_codes_new (id, code, created_by, used_by, created_at, used_at, expires_at)
    SELECT id, code, created_by, used_by, created_at, used_at, expires_at FROM invite_codes;

DROP TABLE invite_codes;

ALTER TABLE invite_codes_new RENAME TO invite_codes;
