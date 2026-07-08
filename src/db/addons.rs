use rusqlite::{params, OptionalExtension};

use super::mods::InstalledFile;
use super::Database;

#[derive(Debug, Clone)]
#[allow(dead_code)] // SQL row model
pub struct InstalledAddon {
    pub id: i64,
    pub forge_addon_id: i64,
    pub parent_mod_id: i64,
    pub forge_version_id: i64,
    pub name: String,
    pub slug: Option<String>,
    pub version: String,
    pub mod_version_constraint: Option<String>,
    pub disabled: bool,
    pub installed_at: String,
    pub updated_at: Option<String>,
}

impl Database {
    #[allow(clippy::too_many_arguments)]
    pub fn insert_addon(
        &self,
        forge_addon_id: i64,
        parent_mod_id: i64,
        forge_version_id: i64,
        name: &str,
        slug: Option<&str>,
        version: &str,
        mod_version_constraint: Option<&str>,
    ) -> rusqlite::Result<i64> {
        self.conn.execute(
            "INSERT INTO installed_addons (forge_addon_id, parent_mod_id, forge_version_id, name, slug, version, mod_version_constraint)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
            params![forge_addon_id, parent_mod_id, forge_version_id, name, slug, version, mod_version_constraint],
        )?;
        Ok(self.conn.last_insert_rowid())
    }

    pub fn get_addon(&self, id: i64) -> rusqlite::Result<Option<InstalledAddon>> {
        self.conn
            .query_row(
                "SELECT id, forge_addon_id, parent_mod_id, forge_version_id, name, slug, version,
                        mod_version_constraint, disabled, installed_at, updated_at
                 FROM installed_addons WHERE id = ?1",
                params![id],
                row_to_installed_addon,
            )
            .optional()
    }

    pub fn get_addon_by_forge_id(
        &self,
        forge_addon_id: i64,
    ) -> rusqlite::Result<Option<InstalledAddon>> {
        self.conn
            .query_row(
                "SELECT id, forge_addon_id, parent_mod_id, forge_version_id, name, slug, version,
                        mod_version_constraint, disabled, installed_at, updated_at
                 FROM installed_addons WHERE forge_addon_id = ?1",
                params![forge_addon_id],
                row_to_installed_addon,
            )
            .optional()
    }

    pub fn get_addon_by_name_or_slug(
        &self,
        query: &str,
    ) -> rusqlite::Result<Option<InstalledAddon>> {
        let by_name = self
            .conn
            .query_row(
                "SELECT id, forge_addon_id, parent_mod_id, forge_version_id, name, slug, version,
                        mod_version_constraint, disabled, installed_at, updated_at
                 FROM installed_addons WHERE LOWER(name) = LOWER(?1)",
                params![query],
                row_to_installed_addon,
            )
            .optional()?;
        if by_name.is_some() {
            return Ok(by_name);
        }
        self.conn
            .query_row(
                "SELECT id, forge_addon_id, parent_mod_id, forge_version_id, name, slug, version,
                        mod_version_constraint, disabled, installed_at, updated_at
                 FROM installed_addons WHERE LOWER(slug) = LOWER(?1)",
                params![query],
                row_to_installed_addon,
            )
            .optional()
    }

    pub fn list_addons(&self) -> rusqlite::Result<Vec<InstalledAddon>> {
        let mut stmt = self.conn.prepare(
            "SELECT id, forge_addon_id, parent_mod_id, forge_version_id, name, slug, version,
                    mod_version_constraint, disabled, installed_at, updated_at
             FROM installed_addons ORDER BY name",
        )?;
        let rows = stmt.query_map([], row_to_installed_addon)?;
        rows.collect()
    }

    pub fn list_addons_for_mod(&self, parent_mod_id: i64) -> rusqlite::Result<Vec<InstalledAddon>> {
        let mut stmt = self.conn.prepare(
            "SELECT id, forge_addon_id, parent_mod_id, forge_version_id, name, slug, version,
                    mod_version_constraint, disabled, installed_at, updated_at
             FROM installed_addons WHERE parent_mod_id = ?1 ORDER BY name",
        )?;
        let rows = stmt.query_map(params![parent_mod_id], row_to_installed_addon)?;
        rows.collect()
    }

    pub fn update_addon(
        &self,
        id: i64,
        forge_version_id: i64,
        version: &str,
        mod_version_constraint: Option<&str>,
    ) -> rusqlite::Result<usize> {
        self.conn.execute(
            "UPDATE installed_addons SET forge_version_id = ?1, version = ?2,
                    mod_version_constraint = ?3, updated_at = datetime('now')
             WHERE id = ?4",
            params![forge_version_id, version, mod_version_constraint, id],
        )
    }

    pub fn delete_addon(&self, id: i64) -> rusqlite::Result<usize> {
        self.conn
            .execute("DELETE FROM installed_addons WHERE id = ?1", params![id])
    }

    pub fn set_addon_disabled(&self, id: i64, disabled: bool) -> rusqlite::Result<usize> {
        self.conn.execute(
            "UPDATE installed_addons SET disabled = ?1 WHERE id = ?2",
            params![disabled as i64, id],
        )
    }

    pub fn count_addons_by_mod(&self) -> rusqlite::Result<std::collections::HashMap<i64, usize>> {
        let mut stmt = self.conn.prepare(
            "SELECT parent_mod_id, COUNT(*) FROM installed_addons GROUP BY parent_mod_id",
        )?;
        let rows = stmt.query_map([], |row| {
            Ok((row.get::<_, i64>(0)?, row.get::<_, i64>(1)? as usize))
        })?;
        let mut map = std::collections::HashMap::new();
        for row in rows {
            let (mod_id, count) = row?;
            map.insert(mod_id, count);
        }
        Ok(map)
    }

    // ── Addon File Tracking ──────────────────────────────────────────

    pub fn insert_addon_file(
        &self,
        addon_id: i64,
        file_path: &str,
        file_hash: Option<&str>,
        file_size: Option<i64>,
    ) -> rusqlite::Result<i64> {
        self.conn.execute(
            "INSERT INTO installed_files (addon_id, file_path, file_hash, file_size)
             VALUES (?1, ?2, ?3, ?4)",
            params![addon_id, file_path, file_hash, file_size],
        )?;
        Ok(self.conn.last_insert_rowid())
    }

    pub fn get_files_for_addon(&self, addon_id: i64) -> rusqlite::Result<Vec<InstalledFile>> {
        let mut stmt = self.conn.prepare(
            "SELECT id, mod_id, addon_id, file_path, file_hash, file_size, source
             FROM installed_files WHERE addon_id = ?1 ORDER BY file_path",
        )?;
        let rows = stmt.query_map(params![addon_id], super::mods::row_to_installed_file)?;
        rows.collect()
    }

    pub fn delete_files_for_addon(&self, addon_id: i64) -> rusqlite::Result<usize> {
        self.conn.execute(
            "DELETE FROM installed_files WHERE addon_id = ?1",
            params![addon_id],
        )
    }
}

fn row_to_installed_addon(row: &rusqlite::Row<'_>) -> rusqlite::Result<InstalledAddon> {
    let disabled_int: i64 = row.get(8)?;
    Ok(InstalledAddon {
        id: row.get(0)?,
        forge_addon_id: row.get(1)?,
        parent_mod_id: row.get(2)?,
        forge_version_id: row.get(3)?,
        name: row.get(4)?,
        slug: row.get(5)?,
        version: row.get(6)?,
        mod_version_constraint: row.get(7)?,
        disabled: disabled_int != 0,
        installed_at: row.get(9)?,
        updated_at: row.get(10)?,
    })
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use crate::db::Database;

    #[test]
    fn insert_and_get_addon() {
        let db = Database::open_in_memory().unwrap();
        let mod_id = db
            .insert_mod(
                Some(100),
                Some(200),
                "ParentMod",
                Some("parent-mod"),
                "1.0.0",
                "forge",
                None,
            )
            .unwrap();
        let addon_id = db
            .insert_addon(
                500,
                mod_id,
                600,
                "TestAddon",
                Some("test-addon"),
                "1.0.0",
                Some("~1.0.0"),
            )
            .unwrap();

        let addon = db.get_addon(addon_id).unwrap().unwrap();
        assert_eq!(addon.forge_addon_id, 500);
        assert_eq!(addon.parent_mod_id, mod_id);
        assert_eq!(addon.name, "TestAddon");
        assert_eq!(addon.mod_version_constraint, Some("~1.0.0".to_string()));
        assert!(!addon.disabled);
    }

    #[test]
    fn get_addon_by_forge_id() {
        let db = Database::open_in_memory().unwrap();
        let mod_id = db
            .insert_mod(
                Some(100),
                Some(200),
                "ParentMod",
                None,
                "1.0.0",
                "forge",
                None,
            )
            .unwrap();
        db.insert_addon(500, mod_id, 600, "TestAddon", None, "1.0.0", None)
            .unwrap();

        let addon = db.get_addon_by_forge_id(500).unwrap().unwrap();
        assert_eq!(addon.name, "TestAddon");
        assert!(db.get_addon_by_forge_id(999).unwrap().is_none());
    }

    #[test]
    fn list_addons_for_mod_returns_only_children() {
        let db = Database::open_in_memory().unwrap();
        let mod1 = db
            .insert_mod(Some(100), Some(200), "Mod1", None, "1.0.0", "forge", None)
            .unwrap();
        let mod2 = db
            .insert_mod(Some(101), Some(201), "Mod2", None, "1.0.0", "forge", None)
            .unwrap();
        db.insert_addon(500, mod1, 600, "Addon1", None, "1.0.0", None)
            .unwrap();
        db.insert_addon(501, mod1, 601, "Addon2", None, "1.0.0", None)
            .unwrap();
        db.insert_addon(502, mod2, 602, "Addon3", None, "1.0.0", None)
            .unwrap();

        let addons = db.list_addons_for_mod(mod1).unwrap();
        assert_eq!(addons.len(), 2);
        assert!(addons.iter().all(|a| a.parent_mod_id == mod1));
    }

    #[test]
    fn delete_addon_cascades_to_files() {
        let db = Database::open_in_memory().unwrap();
        let mod_id = db
            .insert_mod(
                Some(100),
                Some(200),
                "ParentMod",
                None,
                "1.0.0",
                "forge",
                None,
            )
            .unwrap();
        let addon_id = db
            .insert_addon(500, mod_id, 600, "TestAddon", None, "1.0.0", None)
            .unwrap();
        db.insert_addon_file(
            addon_id,
            "BepInEx/plugins/test.dll",
            Some("abc123"),
            Some(1024),
        )
        .unwrap();
        db.insert_addon_file(
            addon_id,
            "BepInEx/plugins/test2.dll",
            Some("def456"),
            Some(2048),
        )
        .unwrap();

        let files = db.get_files_for_addon(addon_id).unwrap();
        assert_eq!(files.len(), 2);

        db.delete_addon(addon_id).unwrap();
        let files = db.get_files_for_addon(addon_id).unwrap();
        assert_eq!(files.len(), 0);
    }

    #[test]
    fn addon_file_has_addon_id_not_mod_id() {
        let db = Database::open_in_memory().unwrap();
        let mod_id = db
            .insert_mod(
                Some(100),
                Some(200),
                "ParentMod",
                None,
                "1.0.0",
                "forge",
                None,
            )
            .unwrap();
        let addon_id = db
            .insert_addon(500, mod_id, 600, "TestAddon", None, "1.0.0", None)
            .unwrap();
        db.insert_addon_file(addon_id, "BepInEx/plugins/addon.dll", None, Some(100))
            .unwrap();

        let files = db.get_files_for_addon(addon_id).unwrap();
        assert_eq!(files.len(), 1);
        assert!(files[0].mod_id.is_none());
        assert_eq!(files[0].addon_id, Some(addon_id));
    }

    #[test]
    fn set_addon_disabled_toggles_flag() {
        let db = Database::open_in_memory().unwrap();
        let mod_id = db
            .insert_mod(
                Some(100),
                Some(200),
                "ParentMod",
                None,
                "1.0.0",
                "forge",
                None,
            )
            .unwrap();
        let addon_id = db
            .insert_addon(500, mod_id, 600, "TestAddon", None, "1.0.0", None)
            .unwrap();

        assert!(!db.get_addon(addon_id).unwrap().unwrap().disabled);
        db.set_addon_disabled(addon_id, true).unwrap();
        assert!(db.get_addon(addon_id).unwrap().unwrap().disabled);
        db.set_addon_disabled(addon_id, false).unwrap();
        assert!(!db.get_addon(addon_id).unwrap().unwrap().disabled);
    }

    #[test]
    fn update_addon_sets_version_and_constraint() {
        let db = Database::open_in_memory().unwrap();
        let mod_id = db
            .insert_mod(
                Some(100),
                Some(200),
                "ParentMod",
                None,
                "1.0.0",
                "forge",
                None,
            )
            .unwrap();
        let addon_id = db
            .insert_addon(500, mod_id, 600, "TestAddon", None, "1.0.0", Some("~1.0.0"))
            .unwrap();

        db.update_addon(addon_id, 601, "2.0.0", Some("~2.0.0"))
            .unwrap();
        let addon = db.get_addon(addon_id).unwrap().unwrap();
        assert_eq!(addon.forge_version_id, 601);
        assert_eq!(addon.version, "2.0.0");
        assert_eq!(addon.mod_version_constraint, Some("~2.0.0".to_string()));
        assert!(addon.updated_at.is_some());
    }
}
