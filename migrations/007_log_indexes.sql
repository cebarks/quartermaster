CREATE INDEX IF NOT EXISTS idx_log_entries_level_id ON log_entries(level, id);
CREATE INDEX IF NOT EXISTS idx_log_entries_target ON log_entries(target);
