-- Request status audit log
CREATE TABLE IF NOT EXISTS request_status_log (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    request_id INTEGER NOT NULL REFERENCES mod_requests(id) ON DELETE CASCADE,
    from_status TEXT NOT NULL,
    to_status TEXT NOT NULL,
    changed_by INTEGER REFERENCES users(id) ON DELETE SET NULL,
    changed_at TEXT NOT NULL DEFAULT (datetime('now')),
    comment TEXT
);

CREATE INDEX IF NOT EXISTS idx_request_status_log_request_id ON request_status_log(request_id);

-- Reclassify existing approved requests that are actually installed
UPDATE mod_requests
SET status = 'installed'
WHERE status = 'approved'
  AND forge_mod_id IN (SELECT forge_mod_id FROM installed_mods WHERE forge_mod_id IS NOT NULL);
