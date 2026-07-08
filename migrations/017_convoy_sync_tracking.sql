-- Convoy sync tracking: download events and client reports

CREATE TABLE convoy_sync_events (
    id          INTEGER PRIMARY KEY,
    event_type  TEXT NOT NULL CHECK(event_type IN ('catalog_fetch', 'catalog_304', 'download')),
    ip          TEXT,
    mod_ids     TEXT,
    bytes_served INTEGER,
    created_at  TEXT NOT NULL DEFAULT (datetime('now'))
);

CREATE TABLE convoy_sync_reports (
    id              INTEGER PRIMARY KEY,
    user_id         INTEGER,
    aid             TEXT NOT NULL,
    result          TEXT NOT NULL CHECK(result IN ('up_to_date', 'updated', 'failed')),
    mods_snapshot   TEXT,
    client_version  TEXT,
    error           TEXT,
    ip              TEXT,
    created_at      TEXT NOT NULL DEFAULT (datetime('now'))
);
