CREATE TABLE headless_session_stats (
    id INTEGER PRIMARY KEY,
    client_index INTEGER NOT NULL,
    profile_id TEXT NOT NULL,
    completed_at TEXT NOT NULL DEFAULT (datetime('now')),
    sent_packets INTEGER NOT NULL,
    sent_data_bytes INTEGER NOT NULL,
    received_packets INTEGER NOT NULL,
    received_data_bytes INTEGER NOT NULL,
    packet_loss_percent REAL NOT NULL,
    time_in_raid_seconds INTEGER NOT NULL
);
