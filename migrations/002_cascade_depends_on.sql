-- Add ON DELETE CASCADE to depends_on_mod_id FK.
-- SQLite doesn't support ALTER CONSTRAINT, so we recreate the table.

CREATE TABLE mod_dependencies_new (
    id                  INTEGER PRIMARY KEY AUTOINCREMENT,
    mod_id              INTEGER NOT NULL REFERENCES installed_mods(id) ON DELETE CASCADE,
    depends_on_mod_id   INTEGER NOT NULL REFERENCES installed_mods(id) ON DELETE CASCADE,
    version_constraint  TEXT,
    UNIQUE(mod_id, depends_on_mod_id)
);

INSERT INTO mod_dependencies_new (id, mod_id, depends_on_mod_id, version_constraint)
    SELECT id, mod_id, depends_on_mod_id, version_constraint FROM mod_dependencies;

DROP TABLE mod_dependencies;

ALTER TABLE mod_dependencies_new RENAME TO mod_dependencies;
