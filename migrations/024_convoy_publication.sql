-- This stamp changes in the same SQLite transaction as the input rows.
CREATE TABLE convoy_publication (
    id INTEGER PRIMARY KEY CHECK (id = 1),
    revision INTEGER NOT NULL CHECK (typeof(revision) = 'integer' AND revision >= 0),
    receipt TEXT,
    receipt_data BLOB,
    receipt_revision INTEGER
);
INSERT INTO convoy_publication (id, revision) VALUES (1, 0);

CREATE TRIGGER convoy_installed_mods_insert
AFTER INSERT ON installed_mods
BEGIN
    UPDATE convoy_publication SET revision = revision + 1 WHERE id = 1;
END;

CREATE TRIGGER convoy_installed_mods_delete
AFTER DELETE ON installed_mods
BEGIN
    UPDATE convoy_publication SET revision = revision + 1 WHERE id = 1;
END;

CREATE TRIGGER convoy_installed_mods_update
AFTER UPDATE OF id, forge_mod_id, forge_version_id, name, version, disabled, group_id, guid ON installed_mods
WHEN OLD.id IS NOT NEW.id
  OR OLD.forge_mod_id IS NOT NEW.forge_mod_id
  OR OLD.forge_version_id IS NOT NEW.forge_version_id
  OR OLD.name IS NOT NEW.name
  OR OLD.version IS NOT NEW.version
  OR OLD.disabled IS NOT NEW.disabled
  OR OLD.group_id IS NOT NEW.group_id
  OR OLD.guid IS NOT NEW.guid
BEGIN
    UPDATE convoy_publication SET revision = revision + 1 WHERE id = 1;
END;

CREATE TRIGGER convoy_installed_files_insert
AFTER INSERT ON installed_files
BEGIN
    UPDATE convoy_publication SET revision = revision + 1 WHERE id = 1;
END;

CREATE TRIGGER convoy_installed_files_delete
AFTER DELETE ON installed_files
BEGIN
    UPDATE convoy_publication SET revision = revision + 1 WHERE id = 1;
END;

CREATE TRIGGER convoy_installed_files_update
AFTER UPDATE OF id, mod_id, addon_id, file_path, file_hash, file_size, source ON installed_files
WHEN OLD.id IS NOT NEW.id
  OR OLD.mod_id IS NOT NEW.mod_id
  OR OLD.addon_id IS NOT NEW.addon_id
  OR OLD.file_path IS NOT NEW.file_path
  OR OLD.file_hash IS NOT NEW.file_hash
  OR OLD.file_size IS NOT NEW.file_size
  OR OLD.source IS NOT NEW.source
BEGIN
    UPDATE convoy_publication SET revision = revision + 1 WHERE id = 1;
END;

CREATE TRIGGER convoy_installed_addons_insert
AFTER INSERT ON installed_addons
BEGIN
    UPDATE convoy_publication SET revision = revision + 1 WHERE id = 1;
END;

CREATE TRIGGER convoy_installed_addons_delete
AFTER DELETE ON installed_addons
BEGIN
    UPDATE convoy_publication SET revision = revision + 1 WHERE id = 1;
END;

CREATE TRIGGER convoy_installed_addons_update
AFTER UPDATE OF id, forge_addon_id, parent_mod_id, forge_version_id, disabled ON installed_addons
WHEN OLD.id IS NOT NEW.id
  OR OLD.forge_addon_id IS NOT NEW.forge_addon_id
  OR OLD.parent_mod_id IS NOT NEW.parent_mod_id
  OR OLD.forge_version_id IS NOT NEW.forge_version_id
  OR OLD.disabled IS NOT NEW.disabled
BEGIN
    UPDATE convoy_publication SET revision = revision + 1 WHERE id = 1;
END;

CREATE TRIGGER convoy_mod_groups_insert
AFTER INSERT ON mod_groups
BEGIN
    UPDATE convoy_publication SET revision = revision + 1 WHERE id = 1;
END;

CREATE TRIGGER convoy_mod_groups_delete
AFTER DELETE ON mod_groups
BEGIN
    UPDATE convoy_publication SET revision = revision + 1 WHERE id = 1;
END;

CREATE TRIGGER convoy_mod_groups_update
AFTER UPDATE OF id, name, slug, tier ON mod_groups
WHEN OLD.id IS NOT NEW.id
  OR OLD.name IS NOT NEW.name
  OR OLD.slug IS NOT NEW.slug
  OR OLD.tier IS NOT NEW.tier
BEGIN
    UPDATE convoy_publication SET revision = revision + 1 WHERE id = 1;
END;
