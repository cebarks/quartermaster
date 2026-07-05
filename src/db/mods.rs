use rusqlite::{params, OptionalExtension};

use super::Database;

#[derive(Debug, Clone)]
#[allow(dead_code)] // SQL row model — fields populated by query results
pub struct InstalledMod {
    pub id: i64,
    pub forge_mod_id: Option<i64>,
    pub forge_version_id: Option<i64>,
    pub name: String,
    pub slug: Option<String>,
    pub version: String,
    pub installed_at: String,
    pub updated_at: Option<String>,
    pub disabled: bool,
    pub source: String,
    pub source_url: Option<String>,
    pub group_id: Option<i64>,
}

#[derive(Debug, Clone)]
#[allow(dead_code)] // SQL row model — fields populated by query results
pub struct ModGroup {
    pub id: i64,
    pub name: String,
    pub slug: String,
    pub tier: String,
    pub exclude_headless: bool,
}

#[derive(Debug, Clone)]
#[allow(dead_code)] // SQL row model
pub struct InstalledFile {
    pub id: i64,
    pub mod_id: Option<i64>,
    pub addon_id: Option<i64>,
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

    #[allow(clippy::too_many_arguments)]
    pub fn insert_mod(
        &self,
        forge_mod_id: Option<i64>,
        forge_version_id: Option<i64>,
        name: &str,
        slug: Option<&str>,
        version: &str,
        source: &str,
        source_url: Option<&str>,
    ) -> rusqlite::Result<i64> {
        self.conn.execute(
            "INSERT INTO installed_mods (forge_mod_id, forge_version_id, name, slug, version, source, source_url)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
            params![forge_mod_id, forge_version_id, name, slug, version, source, source_url],
        )?;
        Ok(self.conn.last_insert_rowid())
    }

    pub fn get_mod(&self, id: i64) -> rusqlite::Result<Option<InstalledMod>> {
        self.conn
            .query_row(
                "SELECT id, forge_mod_id, forge_version_id, name, slug, version, installed_at, updated_at, disabled, source, source_url, group_id
                 FROM installed_mods WHERE id = ?1",
                params![id],
                row_to_installed_mod,
            )
            .optional()
    }

    pub fn get_mod_by_forge_id(&self, forge_mod_id: i64) -> rusqlite::Result<Option<InstalledMod>> {
        self.conn
            .query_row(
                "SELECT id, forge_mod_id, forge_version_id, name, slug, version, installed_at, updated_at, disabled, source, source_url, group_id
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
                "SELECT id, forge_mod_id, forge_version_id, name, slug, version, installed_at, updated_at, disabled, source, source_url, group_id
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
                "SELECT id, forge_mod_id, forge_version_id, name, slug, version, installed_at, updated_at, disabled, source, source_url, group_id
                 FROM installed_mods WHERE LOWER(slug) = LOWER(?1)",
                params![query],
                row_to_installed_mod,
            )
            .optional()
    }

    pub fn list_mods(&self) -> rusqlite::Result<Vec<InstalledMod>> {
        let mut stmt = self.conn.prepare(
            "SELECT id, forge_mod_id, forge_version_id, name, slug, version, installed_at, updated_at, disabled, source, source_url, group_id
             FROM installed_mods ORDER BY name",
        )?;
        let rows = stmt.query_map([], row_to_installed_mod)?;
        rows.collect()
    }

    pub fn list_mods_with_file_counts(&self) -> rusqlite::Result<Vec<(InstalledMod, usize, i64)>> {
        let mut stmt = self.conn.prepare(
            "SELECT m.id, m.forge_mod_id, m.forge_version_id, m.name, m.slug, m.version,
                    m.installed_at, m.updated_at, m.disabled, m.source, m.source_url, m.group_id,
                    COUNT(f.id) as file_count,
                    COALESCE(SUM(f.file_size), 0) as total_size
             FROM installed_mods m
             LEFT JOIN installed_files f ON f.mod_id = m.id
             GROUP BY m.id
             ORDER BY m.name",
        )?;
        let rows = stmt.query_map([], |row| {
            let m = row_to_installed_mod(row)?;
            let count: i64 = row.get(12)?;
            let size: i64 = row.get(13)?;
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
                    m.installed_at, m.updated_at, m.disabled, m.source, m.source_url, m.group_id,
                    COUNT(f.id) as file_count,
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
            let count: i64 = row.get(12)?;
            let size: i64 = row.get(13)?;
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

    pub fn get_files_for_mod(&self, mod_id: i64) -> rusqlite::Result<Vec<InstalledFile>> {
        let mut stmt = self.conn.prepare(
            "SELECT id, mod_id, addon_id, file_path, file_hash, file_size, source
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
            "SELECT f.id, f.mod_id, f.addon_id, f.file_path, f.file_hash, f.file_size, f.source
             FROM installed_files f
             JOIN installed_mods m ON f.mod_id = m.id
             WHERE m.forge_mod_id IN ({placeholders})
             AND m.disabled = 0
             UNION ALL
             SELECT f.id, f.mod_id, f.addon_id, f.file_path, f.file_hash, f.file_size, f.source
             FROM installed_files f
             JOIN installed_addons a ON f.addon_id = a.id
             JOIN installed_mods m ON a.parent_mod_id = m.id
             WHERE m.forge_mod_id IN ({placeholders})
             AND a.disabled = 0 AND m.disabled = 0
             ORDER BY file_path"
        );
        let mut stmt = self.conn.prepare(&sql)?;
        let mut params: Vec<&dyn rusqlite::types::ToSql> = forge_ids
            .iter()
            .map(|id| id as &dyn rusqlite::types::ToSql)
            .collect();
        let params_copy: Vec<&dyn rusqlite::types::ToSql> = forge_ids
            .iter()
            .map(|id| id as &dyn rusqlite::types::ToSql)
            .collect();
        params.extend(params_copy);
        let rows = stmt.query_map(params.as_slice(), row_to_installed_file)?;
        rows.collect()
    }

    pub fn get_all_enabled_mod_files(&self) -> rusqlite::Result<Vec<InstalledFile>> {
        let mut stmt = self.conn.prepare(
            "SELECT f.id, f.mod_id, f.addon_id, f.file_path, f.file_hash, f.file_size, f.source
             FROM installed_files f
             JOIN installed_mods m ON f.mod_id = m.id
             WHERE m.disabled = 0
             UNION ALL
             SELECT f.id, f.mod_id, f.addon_id, f.file_path, f.file_hash, f.file_size, f.source
             FROM installed_files f
             JOIN installed_addons a ON f.addon_id = a.id
             JOIN installed_mods m ON a.parent_mod_id = m.id
             WHERE a.disabled = 0 AND m.disabled = 0
             ORDER BY file_path",
        )?;
        let rows = stmt.query_map([], row_to_installed_file)?;
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
            "SELECT id, mod_id, addon_id, file_path, file_hash, file_size, source
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
        let rows = stmt.query_map([], |row| row.get::<_, Option<i64>>(0))?;
        let mut ids = std::collections::HashSet::new();
        for id in rows {
            if let Some(id) = id? {
                ids.insert(id);
            }
        }
        Ok(ids)
    }

    pub fn count_client_syncable_mods(&self) -> rusqlite::Result<usize> {
        use crate::config::{FIKA_CLIENT_FORGE_ID, FIKA_SERVER_FORGE_ID};
        let sql = format!(
            "SELECT COUNT(DISTINCT m.id) FROM installed_mods m
             JOIN installed_files f ON f.mod_id = m.id
             WHERE f.file_path LIKE 'BepInEx/%'
             AND m.disabled = 0
             AND m.forge_mod_id NOT IN ({}, {})",
            FIKA_CLIENT_FORGE_ID, FIKA_SERVER_FORGE_ID
        );
        let count: i64 = self.conn.query_row(&sql, [], |row| row.get(0))?;
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

    // ── Pending Updates (crash recovery) ─────────────────────────────

    pub fn insert_pending_update(
        &self,
        mod_db_id: i64,
        version_id: i64,
        version_str: &str,
        new_file_paths_json: &str,
        old_file_paths_json: &str,
    ) -> rusqlite::Result<i64> {
        self.conn.execute(
            "INSERT INTO pending_updates (mod_db_id, version_id, version_str, new_file_paths, old_file_paths, item_type)
             VALUES (?1, ?2, ?3, ?4, ?5, 'mod')",
            params![mod_db_id, version_id, version_str, new_file_paths_json, old_file_paths_json],
        )?;
        Ok(self.conn.last_insert_rowid())
    }

    #[allow(dead_code)] // Used in Task 4 (ops.rs)
    pub fn insert_pending_addon_update(
        &self,
        addon_db_id: i64,
        version_id: i64,
        version_str: &str,
        new_file_paths_json: &str,
        old_file_paths_json: &str,
        forge_addon_id: i64,
    ) -> rusqlite::Result<i64> {
        self.conn.execute(
            "INSERT INTO pending_updates (mod_db_id, version_id, version_str, new_file_paths, old_file_paths, item_type, forge_addon_id)
             VALUES (?1, ?2, ?3, ?4, ?5, 'addon', ?6)",
            params![addon_db_id, version_id, version_str, new_file_paths_json, old_file_paths_json, forge_addon_id],
        )?;
        Ok(self.conn.last_insert_rowid())
    }

    pub fn list_pending_updates(&self) -> rusqlite::Result<Vec<PendingUpdate>> {
        let mut stmt = self.conn.prepare(
            "SELECT id, mod_db_id, version_id, version_str, new_file_paths, old_file_paths, started_at,
                    COALESCE(item_type, 'mod') as item_type, forge_addon_id
             FROM pending_updates ORDER BY id",
        )?;
        let rows = stmt.query_map([], |row| {
            Ok(PendingUpdate {
                id: row.get(0)?,
                mod_db_id: row.get(1)?,
                version_id: row.get(2)?,
                version_str: row.get(3)?,
                new_file_paths: row.get(4)?,
                old_file_paths: row.get(5)?,
                started_at: row.get(6)?,
                item_type: row.get(7)?,
                forge_addon_id: row.get(8)?,
            })
        })?;
        rows.collect()
    }

    pub fn delete_pending_update(&self, id: i64) -> rusqlite::Result<usize> {
        self.conn
            .execute("DELETE FROM pending_updates WHERE id = ?1", params![id])
    }

    // ── Convoy Groups ─────────────────────────────────────────────────

    #[allow(dead_code)]
    pub fn list_groups(&self) -> rusqlite::Result<Vec<ModGroup>> {
        let mut stmt = self.conn.prepare(
            "SELECT id, name, slug, tier, exclude_headless FROM mod_groups ORDER BY name",
        )?;
        let rows = stmt.query_map([], |row| {
            Ok(ModGroup {
                id: row.get(0)?,
                name: row.get(1)?,
                slug: row.get(2)?,
                tier: row.get(3)?,
                exclude_headless: row.get(4)?,
            })
        })?;
        rows.collect()
    }

    #[allow(dead_code)]
    pub fn get_group(&self, id: i64) -> rusqlite::Result<Option<ModGroup>> {
        self.conn
            .query_row(
                "SELECT id, name, slug, tier, exclude_headless FROM mod_groups WHERE id = ?1",
                params![id],
                |row| {
                    Ok(ModGroup {
                        id: row.get(0)?,
                        name: row.get(1)?,
                        slug: row.get(2)?,
                        tier: row.get(3)?,
                        exclude_headless: row.get(4)?,
                    })
                },
            )
            .optional()
    }

    #[allow(dead_code)]
    pub fn get_group_by_slug(&self, slug: &str) -> rusqlite::Result<Option<ModGroup>> {
        self.conn
            .query_row(
                "SELECT id, name, slug, tier, exclude_headless FROM mod_groups WHERE slug = ?1",
                params![slug],
                |row| {
                    Ok(ModGroup {
                        id: row.get(0)?,
                        name: row.get(1)?,
                        slug: row.get(2)?,
                        tier: row.get(3)?,
                        exclude_headless: row.get(4)?,
                    })
                },
            )
            .optional()
    }

    #[allow(dead_code)]
    pub fn insert_group(
        &self,
        name: &str,
        slug: &str,
        tier: &str,
        exclude_headless: bool,
    ) -> rusqlite::Result<i64> {
        self.conn.execute(
            "INSERT INTO mod_groups (name, slug, tier, exclude_headless) VALUES (?1, ?2, ?3, ?4)",
            params![name, slug, tier, exclude_headless as i64],
        )?;
        Ok(self.conn.last_insert_rowid())
    }

    #[allow(dead_code)]
    pub fn update_group(
        &self,
        id: i64,
        name: &str,
        slug: &str,
        tier: &str,
        exclude_headless: bool,
    ) -> rusqlite::Result<usize> {
        self.conn.execute(
            "UPDATE mod_groups SET name = ?1, slug = ?2, tier = ?3, exclude_headless = ?4 WHERE id = ?5",
            params![name, slug, tier, exclude_headless as i64, id],
        )
    }

    /// Deletes a group. Explicitly NULLs group_id on all member mods first
    /// because ALTER TABLE ADD COLUMN doesn't enforce FK constraints in SQLite.
    #[allow(dead_code)]
    pub fn delete_group(&self, id: i64) -> rusqlite::Result<usize> {
        self.conn.execute(
            "UPDATE installed_mods SET group_id = NULL WHERE group_id = ?1",
            params![id],
        )?;
        self.conn
            .execute("DELETE FROM mod_groups WHERE id = ?1", params![id])
    }

    #[allow(dead_code)]
    pub fn set_mod_group(&self, mod_id: i64, group_id: Option<i64>) -> rusqlite::Result<usize> {
        self.conn.execute(
            "UPDATE installed_mods SET group_id = ?1 WHERE id = ?2",
            params![group_id, mod_id],
        )
    }

    #[allow(dead_code)]
    pub fn get_mods_in_group(&self, group_id: i64) -> rusqlite::Result<Vec<InstalledMod>> {
        let mut stmt = self.conn.prepare(
            "SELECT id, forge_mod_id, forge_version_id, name, slug, version, installed_at, updated_at, disabled, source, source_url, group_id
             FROM installed_mods WHERE group_id = ?1 ORDER BY name",
        )?;
        let rows = stmt.query_map(params![group_id], row_to_installed_mod)?;
        rows.collect()
    }

    #[allow(dead_code)]
    pub fn get_ungrouped_mods(&self) -> rusqlite::Result<Vec<InstalledMod>> {
        let mut stmt = self.conn.prepare(
            "SELECT id, forge_mod_id, forge_version_id, name, slug, version, installed_at, updated_at, disabled, source, source_url, group_id
             FROM installed_mods WHERE group_id IS NULL ORDER BY name",
        )?;
        let rows = stmt.query_map([], row_to_installed_mod)?;
        rows.collect()
    }

    /// Atomically replace all groups and their memberships.
    /// groups: Vec<(name, slug, tier, exclude_headless, Vec<mod_db_id>)>
    pub fn save_groups_atomic(
        &self,
        groups: &[(String, String, String, bool, Vec<i64>)],
    ) -> anyhow::Result<()> {
        let tx = self.conn.unchecked_transaction()?;

        // Clear all groups
        tx.execute("DELETE FROM mod_groups", [])?;

        // Insert new groups and assign mods
        for (name, slug, tier, exclude_headless, member_ids) in groups {
            tx.execute(
                "INSERT INTO mod_groups (name, slug, tier, exclude_headless) VALUES (?1, ?2, ?3, ?4)",
                params![name, slug, tier, exclude_headless],
            )?;
            let group_id = tx.last_insert_rowid();

            for mod_id in member_ids {
                tx.execute(
                    "UPDATE installed_mods SET group_id = ?1 WHERE id = ?2",
                    params![group_id, mod_id],
                )?;
            }
        }

        tx.commit()?;
        Ok(())
    }
}

/// A record tracking an in-progress async mod/addon update, used for crash recovery.
#[derive(Debug, Clone)]
#[allow(dead_code)] // SQL row model — fields populated by query results
pub struct PendingUpdate {
    pub id: i64,
    pub mod_db_id: i64, // For addons, this is the addon DB ID
    pub version_id: i64,
    pub version_str: String,
    pub new_file_paths: String, // JSON
    pub old_file_paths: String, // JSON
    pub started_at: String,
    pub item_type: String, // "mod" or "addon"
    pub forge_addon_id: Option<i64>,
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
        source: row.get(9)?,
        source_url: row.get(10)?,
        group_id: row.get(11)?,
    })
}

pub(crate) fn row_to_installed_file(row: &rusqlite::Row<'_>) -> rusqlite::Result<InstalledFile> {
    Ok(InstalledFile {
        id: row.get(0)?,
        mod_id: row.get(1)?,
        addon_id: row.get(2)?,
        file_path: row.get(3)?,
        file_hash: row.get(4)?,
        file_size: row.get(5)?,
        source: row.get(6)?,
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
            .insert_mod(
                Some(100),
                Some(200),
                "TestMod",
                Some("test-mod"),
                "1.0.0",
                "forge",
                None,
            )
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
        db.insert_mod(
            Some(100),
            Some(200),
            "EmptyMod",
            None,
            "1.0.0",
            "forge",
            None,
        )
        .unwrap();

        let results = db.list_mods_with_file_counts().unwrap();
        assert_eq!(results.len(), 1);
        let (_, count, size) = &results[0];
        assert_eq!(*count, 0);
        assert_eq!(*size, 0);
    }

    #[test]
    fn set_mod_disabled_toggles_flag() {
        let db = Database::open_in_memory().unwrap();
        let mod_id = db
            .insert_mod(
                Some(100),
                Some(200),
                "TestMod",
                None,
                "1.0.0",
                "forge",
                None,
            )
            .unwrap();

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
        let mod_id = db
            .insert_mod(
                Some(100),
                Some(200),
                "TestMod",
                None,
                "1.0.0",
                "forge",
                None,
            )
            .unwrap();
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
        db.insert_mod(Some(1), Some(1), "Zulu", None, "1.0.0", "forge", None)
            .unwrap();
        db.insert_mod(Some(2), Some(2), "Alpha", None, "2.0.0", "forge", None)
            .unwrap();
        db.insert_mod(Some(3), Some(3), "Mike", None, "3.0.0", "forge", None)
            .unwrap();

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
        db.insert_mod(Some(1), Some(1), "SAIN", None, "1.0.0", "forge", None)
            .unwrap();
        db.insert_mod(Some(2), Some(2), "Big Brain", None, "1.0.0", "forge", None)
            .unwrap();
        db.insert_mod(
            Some(3),
            Some(3),
            "Looting Bots",
            None,
            "1.0.0",
            "forge",
            None,
        )
        .unwrap();

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
        let id1 = db
            .insert_mod(Some(1), Some(1), "EnabledMod", None, "1.0.0", "forge", None)
            .unwrap();
        let id2 = db
            .insert_mod(
                Some(2),
                Some(2),
                "DisabledMod",
                None,
                "1.0.0",
                "forge",
                None,
            )
            .unwrap();
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
        let id1 = db
            .insert_mod(Some(1), Some(1), "Small", None, "1.0.0", "forge", None)
            .unwrap();
        let id2 = db
            .insert_mod(Some(2), Some(2), "Large", None, "1.0.0", "forge", None)
            .unwrap();
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
        db.insert_mod(Some(1), Some(1), "alpha", None, "1.0.0", "forge", None)
            .unwrap();
        db.insert_mod(Some(2), Some(2), "Beta", None, "1.0.0", "forge", None)
            .unwrap();

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

    #[test]
    fn get_all_enabled_mod_files_includes_addon_files() {
        let db = Database::open_in_memory().unwrap();
        db.insert_mod(
            Some(1),
            Some(100),
            "test-mod",
            Some("test-mod"),
            "1.0.0",
            "forge",
            None,
        )
        .unwrap();
        db.insert_file(1, "BepInEx/plugins/test.dll", Some("abc"), Some(100))
            .unwrap();

        db.insert_addon(200, 1, 300, "test-addon", Some("test-addon"), "1.0.0", None)
            .unwrap();
        let addon = db.get_addon_by_forge_id(200).unwrap().unwrap();
        db.insert_addon_file(addon.id, "BepInEx/plugins/addon.dll", Some("def"), Some(50))
            .unwrap();

        let files = db.get_all_enabled_mod_files().unwrap();
        let paths: Vec<&str> = files.iter().map(|f| f.file_path.as_str()).collect();
        assert!(
            paths.contains(&"BepInEx/plugins/test.dll"),
            "should include mod files"
        );
        assert!(
            paths.contains(&"BepInEx/plugins/addon.dll"),
            "should include addon files"
        );
    }
}
