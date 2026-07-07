CREATE TABLE notes (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    author_id INTEGER NOT NULL REFERENCES users(id),
    title TEXT NOT NULL,
    content TEXT NOT NULL,
    visibility TEXT NOT NULL DEFAULT 'private'
        CHECK (visibility IN ('private', 'public_readonly', 'public_editable')),
    pinned INTEGER NOT NULL DEFAULT 0,
    created_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%SZ', 'now')),
    updated_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%SZ', 'now')),
    updated_by INTEGER REFERENCES users(id)
);

CREATE INDEX idx_notes_author ON notes(author_id);
CREATE INDEX idx_notes_visibility ON notes(visibility);

-- Update permission constraint triggers to include notes.edit and notes.manage
DROP TRIGGER IF EXISTS trg_role_permissions_permission_insert;
DROP TRIGGER IF EXISTS trg_role_permissions_permission_update;

CREATE TRIGGER trg_role_permissions_permission_insert
BEFORE INSERT ON role_permissions
BEGIN
    SELECT RAISE(ABORT, 'role_permissions.permission must be a known permission slug')
    WHERE NEW.permission NOT IN (
        'mods.install', 'mods.update', 'mods.remove', 'mods.disable',
        'modsync.manage', 'svm.edit', 'requests.resolve',
        'server.control', 'server.logs', 'server.metrics',
        'headless.manage', 'queue.manage', 'users.manage', 'settings.manage',
        'items.give', 'notes.edit', 'notes.manage'
    );
END;

CREATE TRIGGER trg_role_permissions_permission_update
BEFORE UPDATE OF permission ON role_permissions
BEGIN
    SELECT RAISE(ABORT, 'role_permissions.permission must be a known permission slug')
    WHERE NEW.permission NOT IN (
        'mods.install', 'mods.update', 'mods.remove', 'mods.disable',
        'modsync.manage', 'svm.edit', 'requests.resolve',
        'server.control', 'server.logs', 'server.metrics',
        'headless.manage', 'queue.manage', 'users.manage', 'settings.manage',
        'items.give', 'notes.edit', 'notes.manage'
    );
END;
