-- Quartermaster initial schema

CREATE TABLE IF NOT EXISTS installed_mods (
    id              INTEGER PRIMARY KEY AUTOINCREMENT,
    forge_mod_id    INTEGER NOT NULL UNIQUE,
    forge_version_id INTEGER NOT NULL,
    name            TEXT NOT NULL,
    slug            TEXT NOT NULL,
    version         TEXT NOT NULL,
    installed_at    TEXT NOT NULL DEFAULT (datetime('now')),
    updated_at      TEXT NOT NULL DEFAULT (datetime('now'))
);

CREATE TABLE IF NOT EXISTS installed_files (
    id          INTEGER PRIMARY KEY AUTOINCREMENT,
    mod_id      INTEGER NOT NULL REFERENCES installed_mods(id) ON DELETE CASCADE,
    file_path   TEXT NOT NULL UNIQUE,
    file_hash   TEXT NOT NULL,
    file_size   INTEGER NOT NULL
);

CREATE TABLE IF NOT EXISTS mod_dependencies (
    id                  INTEGER PRIMARY KEY AUTOINCREMENT,
    mod_id              INTEGER NOT NULL REFERENCES installed_mods(id) ON DELETE CASCADE,
    depends_on_mod_id   INTEGER NOT NULL REFERENCES installed_mods(id),
    version_constraint  TEXT,
    UNIQUE(mod_id, depends_on_mod_id)
);

CREATE TABLE IF NOT EXISTS users (
    id              INTEGER PRIMARY KEY AUTOINCREMENT,
    username        TEXT NOT NULL UNIQUE,
    spt_profile_id  TEXT,
    password_hash   TEXT NOT NULL,
    role            TEXT NOT NULL DEFAULT 'player',
    created_at      TEXT NOT NULL DEFAULT (datetime('now'))
);

CREATE TABLE IF NOT EXISTS invite_codes (
    id          INTEGER PRIMARY KEY AUTOINCREMENT,
    code        TEXT NOT NULL UNIQUE,
    created_by  INTEGER NOT NULL REFERENCES users(id),
    used_by     INTEGER REFERENCES users(id),
    created_at  TEXT NOT NULL DEFAULT (datetime('now')),
    used_at     TEXT,
    expires_at  TEXT
);

CREATE TABLE IF NOT EXISTS pending_operations (
    id                  INTEGER PRIMARY KEY AUTOINCREMENT,
    action              TEXT NOT NULL,
    forge_mod_id        INTEGER NOT NULL,
    forge_version_id    INTEGER NOT NULL,
    mod_name            TEXT NOT NULL,
    metadata            TEXT,
    queued_at           TEXT NOT NULL DEFAULT (datetime('now')),
    queued_by           INTEGER REFERENCES users(id)
);
