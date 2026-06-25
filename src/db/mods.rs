use rusqlite::{params, OptionalExtension};

use super::Database;

#[derive(Debug, Clone)]
#[allow(dead_code)] // SQL row model — fields populated by query results
pub struct InstalledMod {
    pub id: i64,
    pub forge_mod_id: i64,
    pub forge_version_id: i64,
    pub name: String,
    pub slug: Option<String>,
    pub version: String,
    pub installed_at: String,
    pub updated_at: Option<String>,
    pub disabled: bool,
}

#[derive(Debug, Clone)]
#[allow(dead_code)] // SQL row model
pub struct InstalledFile {
    pub id: i64,
    pub mod_id: i64,
    pub file_path: String,
    pub file_hash: Option<String>,
    pub file_size: Option<i64>,
    pub source: String,
}

#[derive(Debug, Clone)]
#[allow(dead_code)] // SQL row model
pub struct ModDependency {
    pub id: i64,
    pub mod_id: i64,
    pub depends_on_mod_id: i64,
    pub version_constraint: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ModStatusFilter {
    Enabled,
    Disabled,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ModSortColumn {
    Name,
    Version,
    Files,
    Size,
    Installed,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SortDirection {
    Asc,
    Desc,
}

pub struct ModListFilter {
    pub search: Option<String>,
    pub status: Option<ModStatusFilter>,
    pub sort_column: ModSortColumn,
    pub sort_dir: SortDirection,
}

impl Database {
    // ── Mod CRUD ──────────────────────────────────────────────────────

    pub fn insert_mod(
        &self,
        forge_mod_id: i64,
        forge_version_id: i64,
        name: &str,
        slug: Option<&str>,
        version: &str,
    ) -> rusqlite::Result<i64> {
        self.conn.execute(
            "INSERT INTO installed_mods (forge_mod_id, forge_version_id, name, slug, version)
             VALUES (?1, ?2, ?3, ?4, ?5)",
            params![forge_mod_id, forge_version_id, name, slug, version],
        )?;
        Ok(self.conn.last_insert_rowid())
    }

    pub fn get_mod(&self, id: i64) -> rusqlite::Result<Option<InstalledMod>> {
        self.conn
            .query_row(
                "SELECT id, forge_mod_id, forge_version_id, name, slug, version, installed_at, updated_at, disabled
                 FROM installed_mods WHERE id = ?1",
                params![id],
                row_to_installed_mod,
            )
            .optional()
    }

    pub fn get_mod_by_forge_id(&self, forge_mod_id: i64) -> rusqlite::Result<Option<InstalledMod>> {
        self.conn
            .query_row(
                "SELECT id, forge_mod_id, forge_version_id, name, slug, version, installed_at, updated_at, disabled
                 FROM installed_mods WHERE forge_mod_id = ?1",
                params![forge_mod_id],
                row_to_installed_mod,
            )
            .optional()
    }

    pub fn get_mod_by_name_or_slug(&self, query: &str) -> rusqlite::Result<Option<InstalledMod>> {
        // Name match takes priority over slug match to avoid nondeterminism
        let by_name = self
            .conn
            .query_row(
                "SELECT id, forge_mod_id, forge_version_id, name, slug, version, installed_at, updated_at, disabled
                 FROM installed_mods WHERE LOWER(name) = LOWER(?1)",
                params![query],
                row_to_installed_mod,
            )
            .optional()?;
        if by_name.is_some() {
            return Ok(by_name);
        }
        self.conn
            .query_row(
                "SELECT id, forge_mod_id, forge_version_id, name, slug, version, installed_at, updated_at, disabled
                 FROM installed_mods WHERE LOWER(slug) = LOWER(?1)",
                params![query],
                row_to_installed_mod,
            )
            .optional()
    }

    pub fn list_mods(&self) -> rusqlite::Result<Vec<InstalledMod>> {
        let mut stmt = self.conn.prepare(
            "SELECT id, forge_mod_id, forge_version_id, name, slug, version, installed_at, updated_at, disabled
             FROM installed_mods ORDER BY name",
        )?;
        let rows = stmt.query_map([], row_to_installed_mod)?;
        rows.collect()
    }

    pub fn list_mods_with_file_counts(&self) -> rusqlite::Result<Vec<(InstalledMod, usize, i64)>> {
        let mut stmt = self.conn.prepare(
            "SELECT m.id, m.forge_mod_id, m.forge_version_id, m.name, m.slug, m.version,
                    m.installed_at, m.updated_at, m.disabled, COUNT(f.id) as file_count,
                    COALESCE(SUM(f.file_size), 0) as total_size
             FROM installed_mods m
             LEFT JOIN installed_files f ON f.mod_id = m.id
             GROUP BY m.id
             ORDER BY m.name",
        )?;
        let rows = stmt.query_map([], |row| {
            let m = row_to_installed_mod(row)?;
            let count: i64 = row.get(9)?;
            let size: i64 = row.get(10)?;
            Ok((m, count as usize, size))
        })?;
        rows.collect()
    }

    pub fn list_mods_filtered(
        &self,
        filter: &ModListFilter,
    ) -> rusqlite::Result<Vec<(InstalledMod, usize, i64)>> {
        let mut sql = String::from(
            "SELECT m.id, m.forge_mod_id, m.forge_version_id, m.name, m.slug, m.version,
                    m.installed_at, m.updated_at, m.disabled, COUNT(f.id) as file_count,
                    COALESCE(SUM(f.file_size), 0) as total_size
             FROM installed_mods m
             LEFT JOIN installed_files f ON f.mod_id = m.id",
        );

        let mut conditions: Vec<String> = Vec::new();
        let mut param_values: Vec<Box<dyn rusqlite::types::ToSql>> = Vec::new();

        if let Some(ref search) = filter.search {
            conditions.push(format!(
                "LOWER(m.name) LIKE LOWER('%' || ?{} || '%')",
                param_values.len() + 1
            ));
            param_values.push(Box::new(search.clone()));
        }

        if let Some(status) = filter.status {
            let disabled_val: i64 = match status {
                ModStatusFilter::Enabled => 0,
                ModStatusFilter::Disabled => 1,
            };
            conditions.push(format!("m.disabled = ?{}", param_values.len() + 1));
            param_values.push(Box::new(disabled_val));
        }

        if !conditions.is_empty() {
            sql.push_str(" WHERE ");
            sql.push_str(&conditions.join(" AND "));
        }

        sql.push_str(" GROUP BY m.id ORDER BY ");

        let order_expr = match filter.sort_column {
            ModSortColumn::Name => "LOWER(m.name)",
            ModSortColumn::Version => "m.version",
            ModSortColumn::Files => "file_count",
            ModSortColumn::Size => "total_size",
            ModSortColumn::Installed => "m.installed_at",
        };
        sql.push_str(order_expr);

        match filter.sort_dir {
            SortDirection::Asc => sql.push_str(" ASC"),
            SortDirection::Desc => sql.push_str(" DESC"),
        }

        let mut stmt = self.conn.prepare(&sql)?;
        let params_ref: Vec<&dyn rusqlite::types::ToSql> =
            param_values.iter().map(|p| p.as_ref()).collect();
        let rows = stmt.query_map(params_ref.as_slice(), |row| {
            let m = row_to_installed_mod(row)?;
            let count: i64 = row.get(9)?;
            let size: i64 = row.get(10)?;
            Ok((m, count as usize, size))
        })?;
        rows.collect()
    }

    pub fn update_mod(
        &self,
        id: i64,
        forge_version_id: i64,
        version: &str,
    ) -> rusqlite::Result<usize> {
        self.conn.execute(
            "UPDATE installed_mods SET forge_version_id = ?1, version = ?2, updated_at = datetime('now')
             WHERE id = ?3",
            params![forge_version_id, version, id],
        )
    }

    pub fn delete_mod(&self, id: i64) -> rusqlite::Result<usize> {
        self.conn
            .execute("DELETE FROM installed_mods WHERE id = ?1", params![id])
    }

    // ── File CRUD ─────────────────────────────────────────────────────

    pub fn insert_file(
        &self,
        mod_id: i64,
        file_path: &str,
        file_hash: Option<&str>,
        file_size: Option<i64>,
    ) -> rusqlite::Result<i64> {
        self.conn.execute(
            "INSERT INTO installed_files (mod_id, file_path, file_hash, file_size)
             VALUES (?1, ?2, ?3, ?4)",
            params![mod_id, file_path, file_hash, file_size],
        )?;
        Ok(self.conn.last_insert_rowid())
    }

    pub fn insert_file_with_source(
        &self,
        mod_id: i64,
        file_path: &str,
        file_hash: Option<&str>,
        file_size: Option<i64>,
        source: &str,
    ) -> rusqlite::Result<i64> {
        self.conn.execute(
            "INSERT OR IGNORE INTO installed_files (mod_id, file_path, file_hash, file_size, source)
             VALUES (?1, ?2, ?3, ?4, ?5)",
            params![mod_id, file_path, file_hash, file_size, source],
        )?;
        Ok(self.conn.last_insert_rowid())
    }

    pub fn get_files_for_mod(&self, mod_id: i64) -> rusqlite::Result<Vec<InstalledFile>> {
        let mut stmt = self.conn.prepare(
            "SELECT id, mod_id, file_path, file_hash, file_size, source
             FROM installed_files WHERE mod_id = ?1 ORDER BY file_path",
        )?;
        let rows = stmt.query_map(params![mod_id], row_to_installed_file)?;
        rows.collect()
    }

    pub fn get_files_for_forge_ids(
        &self,
        forge_ids: &[i64],
    ) -> rusqlite::Result<Vec<InstalledFile>> {
        if forge_ids.is_empty() {
            return Ok(Vec::new());
        }
        let placeholders: String = forge_ids.iter().map(|_| "?").collect::<Vec<_>>().join(",");
        let sql = format!(
            "SELECT f.id, f.mod_id, f.file_path, f.file_hash, f.file_size, f.source
             FROM installed_files f
             JOIN installed_mods m ON f.mod_id = m.id
             WHERE m.forge_mod_id IN ({placeholders})
             AND m.disabled = 0
             ORDER BY f.file_path"
        );
        let mut stmt = self.conn.prepare(&sql)?;
        let params: Vec<&dyn rusqlite::types::ToSql> = forge_ids
            .iter()
            .map(|id| id as &dyn rusqlite::types::ToSql)
            .collect();
        let rows = stmt.query_map(params.as_slice(), row_to_installed_file)?;
        rows.collect()
    }

    pub fn delete_files_for_mod(&self, mod_id: i64) -> rusqlite::Result<usize> {
        self.conn.execute(
            "DELETE FROM installed_files WHERE mod_id = ?1",
            params![mod_id],
        )
    }

    pub fn get_all_tracked_files(&self) -> rusqlite::Result<Vec<InstalledFile>> {
        let mut stmt = self.conn.prepare(
            "SELECT id, mod_id, file_path, file_hash, file_size, source
             FROM installed_files ORDER BY file_path",
        )?;
        let rows = stmt.query_map([], row_to_installed_file)?;
        rows.collect()
    }

    pub fn mods_with_server_files(&self) -> rusqlite::Result<std::collections::HashSet<i64>> {
        let mut stmt = self.conn.prepare(
            "SELECT DISTINCT mod_id FROM installed_files
             WHERE file_path LIKE 'SPT/user/mods/%'",
        )?;
        let rows = stmt.query_map([], |row| row.get::<_, i64>(0))?;
        let mut ids = std::collections::HashSet::new();
        for id in rows {
            ids.insert(id?);
        }
        Ok(ids)
    }

    pub fn count_client_syncable_mods(&self) -> rusqlite::Result<usize> {
        let count: i64 = self.conn.query_row(
            "SELECT COUNT(DISTINCT m.id) FROM installed_mods m
             JOIN installed_files f ON f.mod_id = m.id
             WHERE f.file_path LIKE 'BepInEx/%'
             AND m.disabled = 0
             AND m.forge_mod_id NOT IN (2441, 2326, 2357)",
            [],
            |row| row.get(0),
        )?;
        Ok(count as usize)
    }

    // ── Disable/Enable ─────────────────────────────────────────────────

    pub fn set_mod_disabled(&self, id: i64, disabled: bool) -> rusqlite::Result<usize> {
        self.conn.execute(
            "UPDATE installed_mods SET disabled = ?1 WHERE id = ?2",
            params![disabled as i64, id],
        )
    }

    /// Rename a tracked file path in the database (e.g. when disabling/enabling a mod).
    pub fn rename_file_path(&self, file_id: i64, new_path: &str) -> rusqlite::Result<usize> {
        self.conn.execute(
            "UPDATE installed_files SET file_path = ?1 WHERE id = ?2",
            params![new_path, file_id],
        )
    }

    // ── Dependency CRUD ───────────────────────────────────────────────

    pub fn insert_dependency(
        &self,
        mod_id: i64,
        depends_on_mod_id: i64,
        version_constraint: Option<&str>,
    ) -> rusqlite::Result<i64> {
        self.conn.execute(
            "INSERT INTO mod_dependencies (mod_id, depends_on_mod_id, version_constraint)
             VALUES (?1, ?2, ?3)",
            params![mod_id, depends_on_mod_id, version_constraint],
        )?;
        Ok(self.conn.last_insert_rowid())
    }

    pub fn get_dependencies(&self, mod_id: i64) -> rusqlite::Result<Vec<ModDependency>> {
        let mut stmt = self.conn.prepare(
            "SELECT id, mod_id, depends_on_mod_id, version_constraint
             FROM mod_dependencies WHERE mod_id = ?1",
        )?;
        let rows = stmt.query_map(params![mod_id], row_to_mod_dependency)?;
        rows.collect()
    }

    pub fn get_reverse_dependencies(&self, mod_id: i64) -> rusqlite::Result<Vec<ModDependency>> {
        let mut stmt = self.conn.prepare(
            "SELECT id, mod_id, depends_on_mod_id, version_constraint
             FROM mod_dependencies WHERE depends_on_mod_id = ?1",
        )?;
        let rows = stmt.query_map(params![mod_id], row_to_mod_dependency)?;
        rows.collect()
    }

    #[cfg(test)]
    pub fn delete_dependencies_for_mod(&self, mod_id: i64) -> rusqlite::Result<usize> {
        self.conn.execute(
            "DELETE FROM mod_dependencies WHERE mod_id = ?1",
            params![mod_id],
        )
    }
}

fn row_to_installed_mod(row: &rusqlite::Row<'_>) -> rusqlite::Result<InstalledMod> {
    let disabled_int: i64 = row.get(8)?;
    Ok(InstalledMod {
        id: row.get(0)?,
        forge_mod_id: row.get(1)?,
        forge_version_id: row.get(2)?,
        name: row.get(3)?,
        slug: row.get(4)?,
        version: row.get(5)?,
        installed_at: row.get(6)?,
        updated_at: row.get(7)?,
        disabled: disabled_int != 0,
    })
}

fn row_to_installed_file(row: &rusqlite::Row<'_>) -> rusqlite::Result<InstalledFile> {
    Ok(InstalledFile {
        id: row.get(0)?,
        mod_id: row.get(1)?,
        file_path: row.get(2)?,
        file_hash: row.get(3)?,
        file_size: row.get(4)?,
        source: row.get(5)?,
    })
}

fn row_to_mod_dependency(row: &rusqlite::Row<'_>) -> rusqlite::Result<ModDependency> {
    Ok(ModDependency {
        id: row.get(0)?,
        mod_id: row.get(1)?,
        depends_on_mod_id: row.get(2)?,
        version_constraint: row.get(3)?,
    })
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use crate::db::Database;

    #[test]
    fn list_mods_with_file_counts_includes_total_size() {
        let db = Database::open_in_memory().unwrap();
        let mod_id = db
            .insert_mod(100, 200, "TestMod", Some("test-mod"), "1.0.0")
            .unwrap();
        db.insert_file(mod_id, "file1.dll", None, Some(1024))
            .unwrap();
        db.insert_file(mod_id, "file2.dll", None, Some(2048))
            .unwrap();

        let results = db.list_mods_with_file_counts().unwrap();
        assert_eq!(results.len(), 1);
        let (m, count, size) = &results[0];
        assert_eq!(m.name, "TestMod");
        assert_eq!(*count, 2);
        assert_eq!(*size, 3072);
    }

    #[test]
    fn list_mods_with_file_counts_zero_size_when_no_files() {
        let db = Database::open_in_memory().unwrap();
        db.insert_mod(100, 200, "EmptyMod", None, "1.0.0").unwrap();

        let results = db.list_mods_with_file_counts().unwrap();
        assert_eq!(results.len(), 1);
        let (_, count, size) = &results[0];
        assert_eq!(*count, 0);
        assert_eq!(*size, 0);
    }

    #[test]
    fn set_mod_disabled_toggles_flag() {
        let db = Database::open_in_memory().unwrap();
        let mod_id = db.insert_mod(100, 200, "TestMod", None, "1.0.0").unwrap();

        let m = db.get_mod(mod_id).unwrap().unwrap();
        assert!(!m.disabled, "mod should start enabled");

        db.set_mod_disabled(mod_id, true).unwrap();
        let m = db.get_mod(mod_id).unwrap().unwrap();
        assert!(m.disabled, "mod should be disabled");

        db.set_mod_disabled(mod_id, false).unwrap();
        let m = db.get_mod(mod_id).unwrap().unwrap();
        assert!(!m.disabled, "mod should be re-enabled");
    }

    #[test]
    fn rename_file_path_updates_stored_path() {
        let db = Database::open_in_memory().unwrap();
        let mod_id = db.insert_mod(100, 200, "TestMod", None, "1.0.0").unwrap();
        let file_id = db
            .insert_file(mod_id, "SPT/user/mods/TestMod/src/mod.ts", None, Some(100))
            .unwrap();

        db.rename_file_path(file_id, "SPT/user/mods/TestMod.disabled/src/mod.ts")
            .unwrap();
        let files = db.get_files_for_mod(mod_id).unwrap();
        assert_eq!(
            files[0].file_path,
            "SPT/user/mods/TestMod.disabled/src/mod.ts"
        );
    }

    #[test]
    fn list_mods_filtered_default_returns_all_sorted_by_name() {
        let db = Database::open_in_memory().unwrap();
        db.insert_mod(1, 1, "Zulu", None, "1.0.0").unwrap();
        db.insert_mod(2, 2, "Alpha", None, "2.0.0").unwrap();
        db.insert_mod(3, 3, "Mike", None, "3.0.0").unwrap();

        let filter = super::ModListFilter {
            search: None,
            status: None,
            sort_column: super::ModSortColumn::Name,
            sort_dir: super::SortDirection::Asc,
        };
        let results = db.list_mods_filtered(&filter).unwrap();
        assert_eq!(results.len(), 3);
        assert_eq!(results[0].0.name, "Alpha");
        assert_eq!(results[1].0.name, "Mike");
        assert_eq!(results[2].0.name, "Zulu");
    }

    #[test]
    fn list_mods_filtered_search_filters_by_name() {
        let db = Database::open_in_memory().unwrap();
        db.insert_mod(1, 1, "SAIN", None, "1.0.0").unwrap();
        db.insert_mod(2, 2, "Big Brain", None, "1.0.0").unwrap();
        db.insert_mod(3, 3, "Looting Bots", None, "1.0.0").unwrap();

        let filter = super::ModListFilter {
            search: Some("brain".to_string()),
            status: None,
            sort_column: super::ModSortColumn::Name,
            sort_dir: super::SortDirection::Asc,
        };
        let results = db.list_mods_filtered(&filter).unwrap();
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].0.name, "Big Brain");
    }

    #[test]
    fn list_mods_filtered_status_enabled_excludes_disabled() {
        let db = Database::open_in_memory().unwrap();
        let id1 = db.insert_mod(1, 1, "EnabledMod", None, "1.0.0").unwrap();
        let id2 = db.insert_mod(2, 2, "DisabledMod", None, "1.0.0").unwrap();
        db.set_mod_disabled(id2, true).unwrap();
        let _ = id1; // used only for insert

        let filter = super::ModListFilter {
            search: None,
            status: Some(super::ModStatusFilter::Enabled),
            sort_column: super::ModSortColumn::Name,
            sort_dir: super::SortDirection::Asc,
        };
        let results = db.list_mods_filtered(&filter).unwrap();
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].0.name, "EnabledMod");
    }

    #[test]
    fn list_mods_filtered_sort_by_size_desc() {
        let db = Database::open_in_memory().unwrap();
        let id1 = db.insert_mod(1, 1, "Small", None, "1.0.0").unwrap();
        let id2 = db.insert_mod(2, 2, "Large", None, "1.0.0").unwrap();
        db.insert_file(id1, "a.dll", None, Some(100)).unwrap();
        db.insert_file(id2, "b.dll", None, Some(9999)).unwrap();

        let filter = super::ModListFilter {
            search: None,
            status: None,
            sort_column: super::ModSortColumn::Size,
            sort_dir: super::SortDirection::Desc,
        };
        let results = db.list_mods_filtered(&filter).unwrap();
        assert_eq!(results.len(), 2);
        assert_eq!(results[0].0.name, "Large");
        assert_eq!(results[1].0.name, "Small");
    }

    #[test]
    fn list_mods_filtered_sort_name_is_case_insensitive() {
        let db = Database::open_in_memory().unwrap();
        db.insert_mod(1, 1, "alpha", None, "1.0.0").unwrap();
        db.insert_mod(2, 2, "Beta", None, "1.0.0").unwrap();

        let filter = super::ModListFilter {
            search: None,
            status: None,
            sort_column: super::ModSortColumn::Name,
            sort_dir: super::SortDirection::Asc,
        };
        let results = db.list_mods_filtered(&filter).unwrap();
        assert_eq!(results[0].0.name, "alpha");
        assert_eq!(results[1].0.name, "Beta");
    }
}
