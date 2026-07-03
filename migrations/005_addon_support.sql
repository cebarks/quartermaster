-- 1. Addon metadata table
CREATE TABLE installed_addons (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    forge_addon_id INTEGER NOT NULL UNIQUE,
    parent_mod_id INTEGER NOT NULL REFERENCES installed_mods(id) ON DELETE CASCADE,
    forge_version_id INTEGER NOT NULL,
    name TEXT NOT NULL,
    slug TEXT,
    version TEXT NOT NULL,
    mod_version_constraint TEXT,
    disabled INTEGER NOT NULL DEFAULT 0,
    installed_at TEXT NOT NULL DEFAULT (datetime('now')),
    updated_at TEXT
);

CREATE INDEX idx_installed_addons_parent_mod_id ON installed_addons(parent_mod_id);

-- 2. Recreate installed_files with nullable mod_id and new addon_id
-- (SQLite cannot ALTER COLUMN to drop NOT NULL — requires table recreation)
CREATE TABLE installed_files_new (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    mod_id INTEGER REFERENCES installed_mods(id) ON DELETE CASCADE,
    addon_id INTEGER REFERENCES installed_addons(id) ON DELETE CASCADE,
    file_path TEXT NOT NULL UNIQUE,
    file_hash TEXT,
    file_size INTEGER,
    source TEXT NOT NULL DEFAULT 'archive',
    CHECK (
        (mod_id IS NOT NULL AND addon_id IS NULL) OR
        (mod_id IS NULL AND addon_id IS NOT NULL)
    )
);
INSERT INTO installed_files_new (id, mod_id, file_path, file_hash, file_size, source)
    SELECT id, mod_id, file_path, file_hash, file_size, source FROM installed_files;
DROP TABLE installed_files;
ALTER TABLE installed_files_new RENAME TO installed_files;

-- Recreate indexes lost during table recreation (from 001_initial.sql)
CREATE INDEX idx_installed_files_mod_id ON installed_files(mod_id);
CREATE INDEX idx_installed_files_addon_id ON installed_files(addon_id);

-- 3. Recreate pending_operations: make forge_mod_id nullable (addon ops use NULL),
-- add item_type discriminator and forge_addon_id column.
CREATE TABLE pending_operations_new (
    id               INTEGER PRIMARY KEY AUTOINCREMENT,
    action           TEXT NOT NULL,
    forge_mod_id     INTEGER,
    forge_version_id INTEGER,
    mod_name         TEXT NOT NULL,
    metadata         TEXT,
    queued_at        TEXT NOT NULL DEFAULT (datetime('now')),
    queued_by        TEXT,
    item_type        TEXT NOT NULL DEFAULT 'mod',
    forge_addon_id   INTEGER
);
INSERT INTO pending_operations_new (id, action, forge_mod_id, forge_version_id, mod_name, metadata, queued_at, queued_by)
    SELECT id, action, forge_mod_id, forge_version_id, mod_name, metadata, queued_at, queued_by FROM pending_operations;
DROP TABLE pending_operations;
ALTER TABLE pending_operations_new RENAME TO pending_operations;

-- 4. Recreate pending_updates to add item_type and forge_addon_id.
-- Replace UNIQUE(mod_db_id) with UNIQUE(mod_db_id, item_type) to allow separate
-- pending updates for mod and addon with same ID (they're from different tables).
CREATE TABLE pending_updates_new (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    mod_db_id INTEGER NOT NULL,
    version_id INTEGER NOT NULL,
    version_str TEXT NOT NULL,
    new_file_paths TEXT NOT NULL,
    old_file_paths TEXT NOT NULL,
    started_at TEXT NOT NULL DEFAULT (datetime('now')),
    item_type TEXT NOT NULL DEFAULT 'mod',
    forge_addon_id INTEGER,
    UNIQUE(mod_db_id, item_type)
);
INSERT INTO pending_updates_new (id, mod_db_id, version_id, version_str, new_file_paths, old_file_paths, started_at)
    SELECT id, mod_db_id, version_id, version_str, new_file_paths, old_file_paths, started_at FROM pending_updates;
DROP TABLE pending_updates;
ALTER TABLE pending_updates_new RENAME TO pending_updates;
