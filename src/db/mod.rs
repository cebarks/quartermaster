pub mod addons;
pub mod backups;
pub mod logs;
pub mod mods;
pub mod raids;
pub mod rbac;
pub mod requests;
pub mod schema;
pub mod users;

use std::path::Path;

use rusqlite::Connection;

pub struct Database {
    conn: Connection,
}

impl Database {
    /// Open a file-backed database, configure pragmas, and run migrations.
    pub fn open(path: &Path) -> rusqlite::Result<Self> {
        let conn = Connection::open(path)?;
        Self::configure_and_migrate(conn)
    }

    #[allow(dead_code)] // used by integration tests
    pub fn open_in_memory() -> rusqlite::Result<Self> {
        let conn = Connection::open_in_memory()?;
        Self::configure_and_migrate(conn)
    }

    pub fn begin_transaction(&self) -> rusqlite::Result<rusqlite::Transaction<'_>> {
        self.conn.unchecked_transaction()
    }

    #[allow(dead_code)] // used by integration tests
    pub fn conn(&self) -> &Connection {
        &self.conn
    }

    fn configure_and_migrate(conn: Connection) -> rusqlite::Result<Self> {
        conn.pragma_update(None, "journal_mode", "WAL")?;
        conn.pragma_update(None, "busy_timeout", 5000)?;
        conn.pragma_update(None, "foreign_keys", "ON")?;

        schema::run_migrations(&conn)?;
        rbac::sync_builtin_role_permissions(&conn)?;

        Ok(Self { conn })
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests;
