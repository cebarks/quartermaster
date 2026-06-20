CREATE TABLE raids (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    user_id INTEGER NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    spt_profile_id TEXT NOT NULL,
    server_id TEXT,
    player_side TEXT NOT NULL,
    faction TEXT,
    map TEXT NOT NULL,
    time_variant TEXT,
    started_at TEXT NOT NULL,
    ended_at TEXT,
    play_time_seconds INTEGER,
    exit_status TEXT,
    exit_name TEXT,
    killer_id TEXT,
    killer_aid TEXT,
    xp_before INTEGER,
    xp_after INTEGER,
    level_before INTEGER,
    level_after INTEGER,
    victim_count_before INTEGER
);

CREATE INDEX idx_raids_user_id ON raids(user_id);
CREATE INDEX idx_raids_profile_open ON raids(spt_profile_id, ended_at);
CREATE INDEX idx_raids_server_id ON raids(server_id);

CREATE TABLE raid_kills (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    raid_id INTEGER NOT NULL REFERENCES raids(id) ON DELETE CASCADE,
    victim_name TEXT,
    victim_side TEXT,
    victim_role TEXT,
    weapon TEXT,
    distance REAL,
    body_part TEXT,
    kill_time TEXT
);

CREATE INDEX idx_raid_kills_raid_id ON raid_kills(raid_id);

CREATE INDEX IF NOT EXISTS idx_users_spt_profile_id ON users(spt_profile_id);
