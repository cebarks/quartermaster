BEGIN;

CREATE TABLE users_new (
    id              INTEGER PRIMARY KEY AUTOINCREMENT,
    username        TEXT NOT NULL UNIQUE,
    spt_profile_id  TEXT,
    password_hash   TEXT,
    role            TEXT NOT NULL DEFAULT 'player',
    disabled        INTEGER NOT NULL DEFAULT 0,
    created_at      TEXT NOT NULL DEFAULT (datetime('now')),
    password_changed_at TEXT
);

INSERT INTO users_new (id, username, spt_profile_id, password_hash, role, disabled, created_at, password_changed_at)
    SELECT id, username, spt_profile_id, password_hash, role, disabled, created_at, password_changed_at
    FROM users;

DROP TABLE users;
ALTER TABLE users_new RENAME TO users;

COMMIT;
