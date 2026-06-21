BEGIN;

-- Recreate mod_requests with ON DELETE CASCADE on user FKs
CREATE TABLE mod_requests_new (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    user_id INTEGER NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    forge_mod_id INTEGER NOT NULL,
    mod_name TEXT NOT NULL,
    mod_slug TEXT,
    mod_description TEXT,
    fika_compatible TEXT NOT NULL DEFAULT 'unknown',
    reason TEXT,
    status TEXT NOT NULL DEFAULT 'pending',
    resolved_by INTEGER REFERENCES users(id) ON DELETE SET NULL,
    resolved_at TEXT,
    resolve_comment TEXT,
    created_at TEXT NOT NULL DEFAULT (datetime('now')),
    forge_cached_at TEXT NOT NULL DEFAULT (datetime('now'))
);

INSERT INTO mod_requests_new SELECT * FROM mod_requests;
DROP TABLE mod_requests;
ALTER TABLE mod_requests_new RENAME TO mod_requests;

CREATE INDEX idx_mod_requests_status ON mod_requests(status);
CREATE INDEX idx_mod_requests_forge_mod_id ON mod_requests(forge_mod_id);

-- Recreate mod_request_votes with ON DELETE CASCADE on user FK
CREATE TABLE mod_request_votes_new (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    request_id INTEGER NOT NULL REFERENCES mod_requests(id) ON DELETE CASCADE,
    user_id INTEGER NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    upvote INTEGER NOT NULL,
    comment TEXT,
    created_at TEXT NOT NULL DEFAULT (datetime('now')),
    UNIQUE(request_id, user_id)
);

INSERT INTO mod_request_votes_new SELECT * FROM mod_request_votes;
DROP TABLE mod_request_votes;
ALTER TABLE mod_request_votes_new RENAME TO mod_request_votes;

CREATE INDEX idx_mod_request_votes_request_id ON mod_request_votes(request_id);

COMMIT;
