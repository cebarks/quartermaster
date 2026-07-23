CREATE TABLE mod_dependencies_new (
    id                  INTEGER PRIMARY KEY AUTOINCREMENT,
    mod_id              INTEGER NOT NULL REFERENCES installed_mods(id) ON DELETE CASCADE,
    depends_on_mod_id   INTEGER REFERENCES installed_mods(id) ON DELETE SET NULL,
    depends_on_forge_id INTEGER,
    depends_on_name     TEXT,
    version_constraint  TEXT,
    UNIQUE(mod_id, depends_on_forge_id)
);

INSERT INTO mod_dependencies_new (id, mod_id, depends_on_mod_id, version_constraint)
    SELECT id, mod_id, depends_on_mod_id, version_constraint
    FROM mod_dependencies;

DROP TABLE mod_dependencies;
ALTER TABLE mod_dependencies_new RENAME TO mod_dependencies;

UPDATE mod_dependencies
SET depends_on_forge_id = (SELECT forge_mod_id FROM installed_mods WHERE id = depends_on_mod_id),
    depends_on_name = (SELECT name FROM installed_mods WHERE id = depends_on_mod_id)
WHERE depends_on_mod_id IS NOT NULL;
