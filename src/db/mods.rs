use rusqlite::{params, OptionalExtension};

use super::Database;

#[derive(Debug, Clone)]
pub struct InstalledMod {
    pub id: i64,
    pub forge_mod_id: i64,
    pub forge_version_id: i64,
    pub name: String,
    pub slug: String,
    pub version: String,
    pub installed_at: String,
    pub updated_at: String,
}

#[derive(Debug, Clone)]
pub struct InstalledFile {
    pub id: i64,
    pub mod_id: i64,
    pub file_path: String,
    pub file_hash: String,
    pub file_size: i64,
}

#[derive(Debug, Clone)]
pub struct ModDependency {
    pub id: i64,
    pub mod_id: i64,
    pub depends_on_mod_id: i64,
    pub version_constraint: Option<String>,
}

impl Database {
    // ── Mod CRUD ──────────────────────────────────────────────────────

    pub fn insert_mod(
        &self,
        forge_mod_id: i64,
        forge_version_id: i64,
        name: &str,
        slug: &str,
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
                "SELECT id, forge_mod_id, forge_version_id, name, slug, version, installed_at, updated_at
                 FROM installed_mods WHERE id = ?1",
                params![id],
                row_to_installed_mod,
            )
            .optional()
    }

    pub fn get_mod_by_forge_id(&self, forge_mod_id: i64) -> rusqlite::Result<Option<InstalledMod>> {
        self.conn
            .query_row(
                "SELECT id, forge_mod_id, forge_version_id, name, slug, version, installed_at, updated_at
                 FROM installed_mods WHERE forge_mod_id = ?1",
                params![forge_mod_id],
                row_to_installed_mod,
            )
            .optional()
    }

    pub fn list_mods(&self) -> rusqlite::Result<Vec<InstalledMod>> {
        let mut stmt = self.conn.prepare(
            "SELECT id, forge_mod_id, forge_version_id, name, slug, version, installed_at, updated_at
             FROM installed_mods ORDER BY name",
        )?;
        let rows = stmt.query_map([], row_to_installed_mod)?;
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
        file_hash: &str,
        file_size: i64,
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
            "SELECT id, mod_id, file_path, file_hash, file_size
             FROM installed_files WHERE mod_id = ?1 ORDER BY file_path",
        )?;
        let rows = stmt.query_map(params![mod_id], row_to_installed_file)?;
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
            "SELECT id, mod_id, file_path, file_hash, file_size
             FROM installed_files ORDER BY file_path",
        )?;
        let rows = stmt.query_map([], row_to_installed_file)?;
        rows.collect()
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

    pub fn delete_dependencies_for_mod(&self, mod_id: i64) -> rusqlite::Result<usize> {
        self.conn.execute(
            "DELETE FROM mod_dependencies WHERE mod_id = ?1",
            params![mod_id],
        )
    }
}

fn row_to_installed_mod(row: &rusqlite::Row<'_>) -> rusqlite::Result<InstalledMod> {
    Ok(InstalledMod {
        id: row.get(0)?,
        forge_mod_id: row.get(1)?,
        forge_version_id: row.get(2)?,
        name: row.get(3)?,
        slug: row.get(4)?,
        version: row.get(5)?,
        installed_at: row.get(6)?,
        updated_at: row.get(7)?,
    })
}

fn row_to_installed_file(row: &rusqlite::Row<'_>) -> rusqlite::Result<InstalledFile> {
    Ok(InstalledFile {
        id: row.get(0)?,
        mod_id: row.get(1)?,
        file_path: row.get(2)?,
        file_hash: row.get(3)?,
        file_size: row.get(4)?,
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
