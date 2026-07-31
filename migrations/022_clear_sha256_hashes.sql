-- Clear SHA-256 hashes so they get recomputed as xxHash3-64.
-- Integrity checks and convoy catalog skip NULL hashes gracefully.
UPDATE mod_files SET file_hash = NULL;
UPDATE addon_files SET file_hash = NULL;
