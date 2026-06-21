CREATE TABLE raid_snapshots (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    raid_id INTEGER NOT NULL REFERENCES raids(id) ON DELETE CASCADE,
    snapshot_type TEXT NOT NULL,
    data BLOB NOT NULL
);

CREATE UNIQUE INDEX idx_raid_snapshots_raid_type ON raid_snapshots(raid_id, snapshot_type);
