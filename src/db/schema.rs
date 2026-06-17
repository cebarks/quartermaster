use rusqlite::Connection;

const MIGRATION_001: &str = include_str!("../../migrations/001_initial.sql");
const MIGRATION_002: &str = include_str!("../../migrations/002_cascade_depends_on.sql");

pub fn run_migrations(conn: &Connection) -> rusqlite::Result<()> {
    let current_version: i32 = conn.pragma_query_value(None, "user_version", |row| row.get(0))?;

    if current_version < 1 {
        conn.execute_batch(MIGRATION_001)?;
        conn.pragma_update(None, "user_version", 1)?;
    }

    if current_version < 2 {
        conn.execute_batch(MIGRATION_002)?;
        conn.pragma_update(None, "user_version", 2)?;
    }

    Ok(())
}
