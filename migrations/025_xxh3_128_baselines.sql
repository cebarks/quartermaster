-- Preserve prior baselines as pending evidence. They are not XXH3-128 proof.
CREATE TABLE IF NOT EXISTS legacy_file_hashes (
    file_id INTEGER PRIMARY KEY REFERENCES installed_files(id) ON DELETE CASCADE,
    algorithm TEXT NOT NULL,
    hash TEXT
);

INSERT OR IGNORE INTO legacy_file_hashes (file_id, algorithm, hash)
SELECT id,
       CASE length(file_hash) WHEN 16 THEN 'xxh3_64' WHEN 64 THEN 'sha256' ELSE 'unknown' END,
       file_hash
FROM installed_files
WHERE file_hash IS NOT NULL AND length(file_hash) <> 32;

-- Migration 023 had already cleared some legacy values; preserve that pending
-- provenance without misrepresenting genuinely unknown NULL records.
INSERT OR IGNORE INTO legacy_file_hashes (file_id, algorithm, hash)
SELECT file_id, 'unknown', NULL FROM pending_file_rehashes;

-- Legacy values stay intact until a selected, explicit rebaseline succeeds.
INSERT OR IGNORE INTO pending_file_rehashes (file_id)
SELECT file_id FROM legacy_file_hashes;
