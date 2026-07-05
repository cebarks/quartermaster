-- Recreate installed_mods with nullable forge IDs and source tracking
CREATE TABLE installed_mods_new (
    id               INTEGER PRIMARY KEY AUTOINCREMENT,
    forge_mod_id     INTEGER UNIQUE,
    forge_version_id INTEGER,
    name             TEXT NOT NULL,
    slug             TEXT,
    version          TEXT NOT NULL,
    disabled         INTEGER NOT NULL DEFAULT 0,
    installed_at     TEXT NOT NULL DEFAULT (datetime('now')),
    updated_at       TEXT,
    source           TEXT NOT NULL DEFAULT 'forge',
    source_url       TEXT
);

INSERT INTO installed_mods_new (id, forge_mod_id, forge_version_id, name, slug, version, disabled, installed_at, updated_at, source)
    SELECT id, forge_mod_id, forge_version_id, name, slug, version, disabled, installed_at, updated_at, 'forge'
    FROM installed_mods;

DROP TABLE installed_mods;
ALTER TABLE installed_mods_new RENAME TO installed_mods;

-- Add source columns to pending_operations for URL/file queue support
ALTER TABLE pending_operations ADD COLUMN archive_path TEXT;
ALTER TABLE pending_operations ADD COLUMN source TEXT NOT NULL DEFAULT 'forge';
ALTER TABLE pending_operations ADD COLUMN source_url TEXT;
