-- Rebaseline legacy SHA-256 entries from current files without discarding work
-- when a file is unavailable or the process exits before it can be hashed.
CREATE TABLE pending_file_rehashes (
    file_id INTEGER PRIMARY KEY REFERENCES installed_files(id) ON DELETE CASCADE
);

INSERT INTO pending_file_rehashes (file_id)
SELECT id FROM installed_files WHERE length(file_hash) = 64;

UPDATE installed_files SET file_hash = NULL
WHERE id IN (SELECT file_id FROM pending_file_rehashes);
