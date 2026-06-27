-- Track in-progress async mod updates so interrupted updates can be recovered on startup.
-- No FK to installed_mods: we intentionally avoid CASCADE DELETE so the recovery
-- marker survives even if the mod row is somehow removed.

CREATE TABLE IF NOT EXISTS pending_updates (
    id               INTEGER PRIMARY KEY AUTOINCREMENT,
    mod_db_id        INTEGER NOT NULL UNIQUE,  -- only one pending update per mod
    version_id       INTEGER NOT NULL,
    version_str      TEXT NOT NULL,
    new_file_paths   TEXT NOT NULL,  -- JSON: [{path, hash, size}, ...]
    old_file_paths   TEXT NOT NULL,  -- JSON: ["path1", "path2", ...]
    started_at       TEXT NOT NULL DEFAULT (datetime('now'))
);
