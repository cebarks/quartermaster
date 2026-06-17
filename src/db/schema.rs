use rusqlite::Connection;

const MIGRATION_SQL: &str = include_str!("../../migrations/001_initial.sql");

/// Run all pending migrations against the database.
///
/// Uses SQLite's `user_version` pragma to track which migrations have been
/// applied. Currently there is only one migration (version 1).
pub fn run_migrations(conn: &Connection) -> rusqlite::Result<()> {
    let current_version: i32 = conn.pragma_query_value(None, "user_version", |row| row.get(0))?;

    if current_version < 1 {
        conn.execute_batch(MIGRATION_SQL)?;
        conn.pragma_update(None, "user_version", 1)?;
    }

    Ok(())
}
