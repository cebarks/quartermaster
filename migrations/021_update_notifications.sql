-- Remembers which mod updates have already been announced, so a restart (or the
-- next poll) does not re-notify about the same version.
CREATE TABLE IF NOT EXISTS update_notifications (
    mod_db_id   INTEGER NOT NULL,
    version     TEXT    NOT NULL,
    notified_at TEXT    NOT NULL DEFAULT (datetime('now')),
    PRIMARY KEY (mod_db_id, version),
    FOREIGN KEY (mod_db_id) REFERENCES installed_mods(id) ON DELETE CASCADE
);
