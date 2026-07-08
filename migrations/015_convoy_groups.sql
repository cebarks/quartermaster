-- Convoy mod groups
CREATE TABLE mod_groups (
    id               INTEGER PRIMARY KEY AUTOINCREMENT,
    name             TEXT NOT NULL,
    slug             TEXT NOT NULL UNIQUE,
    tier             TEXT NOT NULL DEFAULT 'required' CHECK (tier IN ('required', 'optional')),
    exclude_headless INTEGER NOT NULL DEFAULT 0
);

-- Note: ALTER TABLE ADD COLUMN REFERENCES is decorative in SQLite —
-- FK is not enforced. Referential integrity handled in application code
-- (delete_group() explicitly NULLs group_id before deleting).
ALTER TABLE installed_mods ADD COLUMN group_id INTEGER;
