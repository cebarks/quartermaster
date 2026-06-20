CREATE TABLE mod_requests (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    user_id INTEGER NOT NULL REFERENCES users(id),
    forge_mod_id INTEGER NOT NULL,
    mod_name TEXT NOT NULL,
    mod_slug TEXT,
    mod_description TEXT,
    fika_compatible TEXT NOT NULL DEFAULT 'unknown',
    reason TEXT,
    status TEXT NOT NULL DEFAULT 'pending',
    resolved_by INTEGER REFERENCES users(id),
    resolved_at TEXT,
    resolve_comment TEXT,
    created_at TEXT NOT NULL DEFAULT (datetime('now')),
    forge_cached_at TEXT NOT NULL DEFAULT (datetime('now'))
);

CREATE TABLE mod_request_votes (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    request_id INTEGER NOT NULL REFERENCES mod_requests(id) ON DELETE CASCADE,
    user_id INTEGER NOT NULL REFERENCES users(id),
    upvote INTEGER NOT NULL,
    comment TEXT,
    created_at TEXT NOT NULL DEFAULT (datetime('now')),
    UNIQUE(request_id, user_id)
);

CREATE INDEX idx_mod_requests_status ON mod_requests(status);
CREATE INDEX idx_mod_requests_forge_mod_id ON mod_requests(forge_mod_id);
CREATE INDEX idx_mod_request_votes_request_id ON mod_request_votes(request_id);
