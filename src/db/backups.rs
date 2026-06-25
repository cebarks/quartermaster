use rusqlite::{params, OptionalExtension};

use super::Database;

#[derive(Debug, Clone)]
pub struct BackupRecord {
    pub id: i64,
    pub backup_type: String,
    pub trigger: String,
    pub backup_id: String,
    #[allow(dead_code)] // SQL row model field — used in tests and future UI
    pub mod_db_id: Option<i64>,
    pub forge_mod_id: Option<i64>,
    pub forge_version_id: Option<i64>,
    pub mod_name: Option<String>,
    pub mod_slug: Option<String>,
    pub mod_version: Option<String>,
    pub backup_path: String,
    pub backup_size: Option<i64>,
    pub created_at: String,
    #[allow(dead_code)] // SQL row model field — used in tests
    pub restored_at: Option<String>,
}

fn row_to_backup(row: &rusqlite::Row) -> rusqlite::Result<BackupRecord> {
    Ok(BackupRecord {
        id: row.get(0)?,
        backup_type: row.get(1)?,
        trigger: row.get(2)?,
        backup_id: row.get(3)?,
        mod_db_id: row.get(4)?,
        forge_mod_id: row.get(5)?,
        forge_version_id: row.get(6)?,
        mod_name: row.get(7)?,
        mod_slug: row.get(8)?,
        mod_version: row.get(9)?,
        backup_path: row.get(10)?,
        backup_size: row.get(11)?,
        created_at: row.get(12)?,
        restored_at: row.get(13)?,
    })
}

const SELECT_COLS: &str = "id, backup_type, trigger, backup_id, mod_db_id, forge_mod_id, \
    forge_version_id, mod_name, mod_slug, mod_version, backup_path, backup_size, \
    created_at, restored_at";

impl Database {
    #[allow(clippy::too_many_arguments)]
    pub fn insert_backup(
        &self,
        backup_type: &str,
        trigger: &str,
        backup_id: &str,
        mod_db_id: Option<i64>,
        forge_mod_id: Option<i64>,
        forge_version_id: Option<i64>,
        mod_name: Option<&str>,
        mod_slug: Option<&str>,
        mod_version: Option<&str>,
        backup_path: &str,
        backup_size: Option<i64>,
    ) -> rusqlite::Result<i64> {
        self.conn.execute(
            "INSERT INTO backups (backup_type, trigger, backup_id, mod_db_id, forge_mod_id,
             forge_version_id, mod_name, mod_slug, mod_version, backup_path, backup_size)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11)",
            params![
                backup_type,
                trigger,
                backup_id,
                mod_db_id,
                forge_mod_id,
                forge_version_id,
                mod_name,
                mod_slug,
                mod_version,
                backup_path,
                backup_size
            ],
        )?;
        Ok(self.conn.last_insert_rowid())
    }

    pub fn get_backup(&self, id: i64) -> rusqlite::Result<Option<BackupRecord>> {
        self.conn
            .query_row(
                &format!("SELECT {SELECT_COLS} FROM backups WHERE id = ?1"),
                params![id],
                row_to_backup,
            )
            .optional()
    }

    pub fn list_backups_for_mod(&self, forge_mod_id: i64) -> rusqlite::Result<Vec<BackupRecord>> {
        let mut stmt = self.conn.prepare(&format!(
            "SELECT {SELECT_COLS} FROM backups WHERE forge_mod_id = ?1 ORDER BY created_at DESC, id DESC"
        ))?;
        let rows = stmt.query_map(params![forge_mod_id], row_to_backup)?;
        rows.collect()
    }

    pub fn list_all_backups(&self) -> rusqlite::Result<Vec<BackupRecord>> {
        let mut stmt = self.conn.prepare(&format!(
            "SELECT {SELECT_COLS} FROM backups ORDER BY created_at DESC, id DESC"
        ))?;
        let rows = stmt.query_map([], row_to_backup)?;
        rows.collect()
    }

    pub fn get_latest_backup_for_mod(
        &self,
        forge_mod_id: i64,
    ) -> rusqlite::Result<Option<BackupRecord>> {
        self.conn
            .query_row(
                &format!("SELECT {SELECT_COLS} FROM backups WHERE forge_mod_id = ?1 ORDER BY created_at DESC, id DESC LIMIT 1"),
                params![forge_mod_id],
                row_to_backup,
            )
            .optional()
    }

    pub fn set_backup_restored(&self, id: i64) -> rusqlite::Result<()> {
        self.conn.execute(
            "UPDATE backups SET restored_at = datetime('now') WHERE id = ?1",
            params![id],
        )?;
        Ok(())
    }

    pub fn delete_backup(&self, id: i64) -> rusqlite::Result<()> {
        self.conn
            .execute("DELETE FROM backups WHERE id = ?1", params![id])?;
        Ok(())
    }

    pub fn count_backups_for_mod(&self, forge_mod_id: i64) -> rusqlite::Result<i64> {
        self.conn.query_row(
            "SELECT COUNT(*) FROM backups WHERE backup_type = 'mod' AND forge_mod_id = ?1",
            params![forge_mod_id],
            |row| row.get(0),
        )
    }

    pub fn count_full_backups(&self) -> rusqlite::Result<i64> {
        self.conn.query_row(
            "SELECT COUNT(*) FROM backups WHERE backup_type = 'full'",
            [],
            |row| row.get(0),
        )
    }

    pub fn oldest_backup_for_mod(
        &self,
        forge_mod_id: i64,
    ) -> rusqlite::Result<Option<BackupRecord>> {
        self.conn
            .query_row(
                &format!("SELECT {SELECT_COLS} FROM backups WHERE backup_type = 'mod' AND forge_mod_id = ?1 ORDER BY created_at ASC, id ASC LIMIT 1"),
                params![forge_mod_id],
                row_to_backup,
            )
            .optional()
    }

    pub fn oldest_full_backup(&self) -> rusqlite::Result<Option<BackupRecord>> {
        self.conn
            .query_row(
                &format!("SELECT {SELECT_COLS} FROM backups WHERE backup_type = 'full' ORDER BY created_at ASC, id ASC LIMIT 1"),
                params![],
                row_to_backup,
            )
            .optional()
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use crate::db::Database;

    #[test]
    fn insert_and_get_backup() {
        let db = Database::open_in_memory().unwrap();
        let id = db
            .insert_backup(
                "mod",
                "manual",
                "20260624T143000",
                None,
                Some(100),
                Some(200),
                Some("TestMod"),
                Some("test-mod"),
                Some("1.0.0"),
                "quartermaster/backups/mods/100/20260624T143000",
                Some(1024),
            )
            .unwrap();
        let backup = db.get_backup(id).unwrap().unwrap();
        assert_eq!(backup.backup_type, "mod");
        assert_eq!(backup.trigger, "manual");
        assert_eq!(backup.forge_mod_id, Some(100));
        assert_eq!(backup.mod_name.as_deref(), Some("TestMod"));
        assert_eq!(backup.backup_size, Some(1024));
    }

    #[test]
    fn list_backups_for_mod_filtered() {
        let db = Database::open_in_memory().unwrap();
        db.insert_backup(
            "mod",
            "manual",
            "20260624T140000",
            None,
            Some(100),
            Some(200),
            Some("Mod1"),
            None,
            Some("1.0"),
            "path/1",
            Some(100),
        )
        .unwrap();
        db.insert_backup(
            "mod",
            "manual",
            "20260624T150000",
            None,
            Some(200),
            Some(300),
            Some("Mod2"),
            None,
            Some("2.0"),
            "path/2",
            Some(200),
        )
        .unwrap();
        let backups = db.list_backups_for_mod(100).unwrap();
        assert_eq!(backups.len(), 1);
        assert_eq!(backups[0].mod_name.as_deref(), Some("Mod1"));
    }

    #[test]
    fn set_backup_restored() {
        let db = Database::open_in_memory().unwrap();
        let id = db
            .insert_backup(
                "mod",
                "manual",
                "20260624T143000",
                None,
                Some(100),
                Some(200),
                Some("TestMod"),
                None,
                Some("1.0"),
                "path/1",
                None,
            )
            .unwrap();
        assert!(db.get_backup(id).unwrap().unwrap().restored_at.is_none());
        db.set_backup_restored(id).unwrap();
        assert!(db.get_backup(id).unwrap().unwrap().restored_at.is_some());
    }

    #[test]
    fn delete_backup() {
        let db = Database::open_in_memory().unwrap();
        let id = db
            .insert_backup(
                "mod",
                "manual",
                "20260624T143000",
                None,
                Some(100),
                Some(200),
                Some("TestMod"),
                None,
                Some("1.0"),
                "path/1",
                None,
            )
            .unwrap();
        db.delete_backup(id).unwrap();
        assert!(db.get_backup(id).unwrap().is_none());
    }

    #[test]
    fn count_and_oldest_backup() {
        let db = Database::open_in_memory().unwrap();
        db.insert_backup(
            "mod",
            "manual",
            "20260624T140000",
            None,
            Some(100),
            Some(200),
            Some("Mod"),
            None,
            Some("1.0"),
            "path/1",
            None,
        )
        .unwrap();
        db.insert_backup(
            "mod",
            "manual",
            "20260624T150000",
            None,
            Some(100),
            Some(200),
            Some("Mod"),
            None,
            Some("2.0"),
            "path/2",
            None,
        )
        .unwrap();
        assert_eq!(db.count_backups_for_mod(100).unwrap(), 2);
        let oldest = db.oldest_backup_for_mod(100).unwrap().unwrap();
        assert_eq!(oldest.backup_id, "20260624T140000");
    }

    #[test]
    fn full_backup_count_separate_from_mod() {
        let db = Database::open_in_memory().unwrap();
        db.insert_backup(
            "mod",
            "manual",
            "20260624T140000",
            None,
            Some(100),
            Some(200),
            Some("Mod"),
            None,
            Some("1.0"),
            "path/1",
            None,
        )
        .unwrap();
        db.insert_backup(
            "full",
            "manual",
            "20260624T150000",
            None,
            None,
            None,
            None,
            None,
            None,
            "path/full/1",
            None,
        )
        .unwrap();
        assert_eq!(db.count_backups_for_mod(100).unwrap(), 1);
        assert_eq!(db.count_full_backups().unwrap(), 1);
    }

    #[test]
    fn get_latest_backup_for_mod() {
        let db = Database::open_in_memory().unwrap();
        db.insert_backup(
            "mod",
            "manual",
            "20260624T140000",
            None,
            Some(100),
            Some(200),
            Some("Mod"),
            None,
            Some("1.0"),
            "path/1",
            None,
        )
        .unwrap();
        db.insert_backup(
            "mod",
            "manual",
            "20260624T150000",
            None,
            Some(100),
            Some(200),
            Some("Mod"),
            None,
            Some("2.0"),
            "path/2",
            None,
        )
        .unwrap();
        let latest = db.get_latest_backup_for_mod(100).unwrap().unwrap();
        assert_eq!(latest.mod_version.as_deref(), Some("2.0"));
    }

    #[test]
    fn backup_survives_mod_deletion() {
        let db = Database::open_in_memory().unwrap();
        let mod_id = db
            .insert_mod(100, 200, "TestMod", Some("test-mod"), "1.0.0")
            .unwrap();
        db.insert_backup(
            "mod",
            "manual",
            "20260624T143000",
            Some(mod_id),
            Some(100),
            Some(200),
            Some("TestMod"),
            Some("test-mod"),
            Some("1.0.0"),
            "path/1",
            None,
        )
        .unwrap();
        db.delete_mod(mod_id).unwrap();
        let backups = db.list_backups_for_mod(100).unwrap();
        assert_eq!(backups.len(), 1);
        assert!(backups[0].mod_db_id.is_none());
        assert_eq!(backups[0].mod_name.as_deref(), Some("TestMod"));
    }
}
