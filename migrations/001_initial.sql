-- Quartermaster schema

CREATE TABLE IF NOT EXISTS installed_mods (
    id               INTEGER PRIMARY KEY AUTOINCREMENT,
    forge_mod_id     INTEGER NOT NULL UNIQUE,
    forge_version_id INTEGER NOT NULL,
    name             TEXT NOT NULL,
    slug             TEXT,
    version          TEXT NOT NULL,
    disabled         INTEGER NOT NULL DEFAULT 0,
    installed_at     TEXT NOT NULL DEFAULT (datetime('now')),
    updated_at       TEXT
);

CREATE TABLE IF NOT EXISTS installed_files (
    id        INTEGER PRIMARY KEY AUTOINCREMENT,
    mod_id    INTEGER NOT NULL REFERENCES installed_mods(id) ON DELETE CASCADE,
    file_path TEXT NOT NULL UNIQUE,
    file_hash TEXT,
    file_size INTEGER,
    source    TEXT NOT NULL DEFAULT 'archive'
);

CREATE INDEX IF NOT EXISTS idx_installed_files_mod_id ON installed_files(mod_id);

CREATE TABLE IF NOT EXISTS mod_dependencies (
    id                 INTEGER PRIMARY KEY AUTOINCREMENT,
    mod_id             INTEGER NOT NULL REFERENCES installed_mods(id) ON DELETE CASCADE,
    depends_on_mod_id  INTEGER NOT NULL REFERENCES installed_mods(id) ON DELETE CASCADE,
    version_constraint TEXT,
    UNIQUE(mod_id, depends_on_mod_id)
);

CREATE TABLE IF NOT EXISTS users (
    id                  INTEGER PRIMARY KEY AUTOINCREMENT,
    username            TEXT NOT NULL UNIQUE,
    spt_profile_id      TEXT,
    password_hash       TEXT,
    role                TEXT NOT NULL DEFAULT 'player',
    disabled            INTEGER NOT NULL DEFAULT 0,
    created_at          TEXT NOT NULL DEFAULT (datetime('now')),
    password_changed_at TEXT
);

CREATE INDEX IF NOT EXISTS idx_users_spt_profile_id ON users(spt_profile_id);

CREATE TABLE IF NOT EXISTS invite_codes (
    id         INTEGER PRIMARY KEY AUTOINCREMENT,
    code       TEXT NOT NULL UNIQUE,
    created_by INTEGER REFERENCES users(id),
    used_by    INTEGER REFERENCES users(id),
    created_at TEXT NOT NULL DEFAULT (datetime('now')),
    used_at    TEXT,
    expires_at TEXT
);

CREATE TABLE IF NOT EXISTS password_reset_tokens (
    id         INTEGER PRIMARY KEY AUTOINCREMENT,
    user_id    INTEGER NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    token      TEXT NOT NULL UNIQUE,
    expires_at TEXT NOT NULL,
    created_at TEXT NOT NULL DEFAULT (datetime('now'))
);

CREATE TABLE IF NOT EXISTS pending_operations (
    id               INTEGER PRIMARY KEY AUTOINCREMENT,
    action           TEXT NOT NULL,
    forge_mod_id     INTEGER NOT NULL,
    forge_version_id INTEGER,
    mod_name         TEXT NOT NULL,
    metadata         TEXT,
    queued_at        TEXT NOT NULL DEFAULT (datetime('now')),
    queued_by        TEXT
);

CREATE TABLE IF NOT EXISTS mod_requests (
    id              INTEGER PRIMARY KEY AUTOINCREMENT,
    user_id         INTEGER NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    forge_mod_id    INTEGER NOT NULL,
    mod_name        TEXT NOT NULL,
    mod_slug        TEXT,
    mod_description TEXT,
    fika_compatible TEXT NOT NULL DEFAULT 'unknown',
    reason          TEXT,
    status          TEXT NOT NULL DEFAULT 'pending',
    resolved_by     INTEGER REFERENCES users(id) ON DELETE SET NULL,
    resolved_at     TEXT,
    resolve_comment TEXT,
    created_at      TEXT NOT NULL DEFAULT (datetime('now')),
    forge_cached_at TEXT NOT NULL DEFAULT (datetime('now'))
);

CREATE INDEX IF NOT EXISTS idx_mod_requests_status ON mod_requests(status);
CREATE INDEX IF NOT EXISTS idx_mod_requests_forge_mod_id ON mod_requests(forge_mod_id);

CREATE TABLE IF NOT EXISTS mod_request_votes (
    id         INTEGER PRIMARY KEY AUTOINCREMENT,
    request_id INTEGER NOT NULL REFERENCES mod_requests(id) ON DELETE CASCADE,
    user_id    INTEGER NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    upvote     INTEGER NOT NULL,
    comment    TEXT,
    created_at TEXT NOT NULL DEFAULT (datetime('now')),
    UNIQUE(request_id, user_id)
);

CREATE INDEX IF NOT EXISTS idx_mod_request_votes_request_id ON mod_request_votes(request_id);

CREATE TABLE IF NOT EXISTS raids (
    id                  INTEGER PRIMARY KEY AUTOINCREMENT,
    user_id             INTEGER NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    spt_profile_id      TEXT NOT NULL,
    server_id           TEXT,
    player_side         TEXT NOT NULL,
    faction             TEXT,
    map                 TEXT NOT NULL,
    time_variant        TEXT,
    started_at          TEXT NOT NULL,
    ended_at            TEXT,
    play_time_seconds   INTEGER,
    exit_status         TEXT,
    exit_name           TEXT,
    killer_id           TEXT,
    killer_aid          TEXT,
    xp_before           INTEGER,
    xp_after            INTEGER,
    level_before        INTEGER,
    level_after         INTEGER,
    victim_count_before INTEGER
);

CREATE INDEX IF NOT EXISTS idx_raids_user_id ON raids(user_id);
CREATE INDEX IF NOT EXISTS idx_raids_profile_open ON raids(spt_profile_id, ended_at);
CREATE INDEX IF NOT EXISTS idx_raids_server_id ON raids(server_id);

CREATE TABLE IF NOT EXISTS raid_kills (
    id          INTEGER PRIMARY KEY AUTOINCREMENT,
    raid_id     INTEGER NOT NULL REFERENCES raids(id) ON DELETE CASCADE,
    victim_name TEXT,
    victim_side TEXT,
    victim_role TEXT,
    weapon      TEXT,
    distance    REAL,
    body_part   TEXT,
    kill_time   TEXT
);

CREATE INDEX IF NOT EXISTS idx_raid_kills_raid_id ON raid_kills(raid_id);

CREATE TABLE IF NOT EXISTS raid_snapshots (
    id            INTEGER PRIMARY KEY AUTOINCREMENT,
    raid_id       INTEGER NOT NULL REFERENCES raids(id) ON DELETE CASCADE,
    snapshot_type TEXT NOT NULL,
    data          BLOB NOT NULL
);

CREATE UNIQUE INDEX IF NOT EXISTS idx_raid_snapshots_raid_type ON raid_snapshots(raid_id, snapshot_type);

CREATE TABLE IF NOT EXISTS roles (
    id           INTEGER PRIMARY KEY AUTOINCREMENT,
    name         TEXT NOT NULL UNIQUE,
    display_name TEXT NOT NULL,
    built_in     INTEGER NOT NULL DEFAULT 0,
    created_at   TEXT NOT NULL DEFAULT (datetime('now'))
);

CREATE TABLE IF NOT EXISTS role_permissions (
    role_id    INTEGER NOT NULL REFERENCES roles(id) ON DELETE CASCADE,
    permission TEXT NOT NULL,
    PRIMARY KEY (role_id, permission)
);

INSERT OR IGNORE INTO roles (name, display_name, built_in) VALUES ('admin', 'Admin', 1);
INSERT OR IGNORE INTO roles (name, display_name, built_in) VALUES ('moderator', 'Moderator', 1);
INSERT OR IGNORE INTO roles (name, display_name, built_in) VALUES ('player', 'Player', 1);

INSERT OR IGNORE INTO role_permissions (role_id, permission)
SELECT r.id, p.permission
FROM roles r,
     (SELECT 'mods.install' AS permission
      UNION ALL SELECT 'mods.update'
      UNION ALL SELECT 'mods.remove'
      UNION ALL SELECT 'mods.disable'
      UNION ALL SELECT 'modsync.manage'
      UNION ALL SELECT 'svm.edit'
      UNION ALL SELECT 'requests.resolve'
      UNION ALL SELECT 'server.control'
      UNION ALL SELECT 'server.logs'
      UNION ALL SELECT 'server.metrics'
      UNION ALL SELECT 'headless.manage'
      UNION ALL SELECT 'queue.manage'
      UNION ALL SELECT 'users.manage'
      UNION ALL SELECT 'settings.manage') p
WHERE r.name = 'admin';

INSERT OR IGNORE INTO role_permissions (role_id, permission)
SELECT r.id, p.permission
FROM roles r,
     (SELECT 'mods.install' AS permission
      UNION ALL SELECT 'mods.update'
      UNION ALL SELECT 'mods.remove'
      UNION ALL SELECT 'mods.disable'
      UNION ALL SELECT 'modsync.manage'
      UNION ALL SELECT 'svm.edit'
      UNION ALL SELECT 'requests.resolve'
      UNION ALL SELECT 'server.control'
      UNION ALL SELECT 'server.logs'
      UNION ALL SELECT 'server.metrics'
      UNION ALL SELECT 'headless.manage'
      UNION ALL SELECT 'queue.manage') p
WHERE r.name = 'moderator';

CREATE TABLE IF NOT EXISTS backups (
    id               INTEGER PRIMARY KEY,
    backup_type      TEXT NOT NULL CHECK (backup_type IN ('mod', 'full')),
    trigger          TEXT NOT NULL CHECK (trigger IN ('auto_update', 'auto_remove', 'auto_disable', 'auto_enable', 'manual')),
    backup_id        TEXT NOT NULL,
    mod_db_id        INTEGER,
    forge_mod_id     INTEGER,
    forge_version_id INTEGER,
    mod_name         TEXT,
    mod_slug         TEXT,
    mod_version      TEXT,
    backup_path      TEXT NOT NULL,
    backup_size      INTEGER,
    created_at       TEXT NOT NULL DEFAULT (datetime('now')),
    restored_at      TEXT,
    FOREIGN KEY (mod_db_id) REFERENCES installed_mods(id) ON DELETE SET NULL
);
