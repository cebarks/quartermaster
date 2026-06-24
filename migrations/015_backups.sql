CREATE TABLE backups (
    id INTEGER PRIMARY KEY,
    backup_type TEXT NOT NULL CHECK (backup_type IN ('mod', 'full')),
    trigger TEXT NOT NULL CHECK (trigger IN ('auto_update', 'auto_remove', 'auto_disable', 'auto_enable', 'manual')),
    backup_id TEXT NOT NULL,
    mod_db_id INTEGER,
    forge_mod_id INTEGER,
    forge_version_id INTEGER,
    mod_name TEXT,
    mod_slug TEXT,
    mod_version TEXT,
    backup_path TEXT NOT NULL,
    backup_size INTEGER,
    created_at TEXT NOT NULL DEFAULT (datetime('now')),
    restored_at TEXT,
    FOREIGN KEY (mod_db_id) REFERENCES installed_mods(id) ON DELETE SET NULL
);
