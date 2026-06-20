use rusqlite::Connection;

const MIGRATION_001: &str = include_str!("../../migrations/001_initial.sql");
const MIGRATION_002: &str = include_str!("../../migrations/002_cascade_depends_on.sql");
const MIGRATION_003: &str = include_str!("../../migrations/003_file_mod_id_index.sql");
const MIGRATION_004: &str = include_str!("../../migrations/004_file_source.sql");
const MIGRATION_005: &str = include_str!("../../migrations/005_add_disabled_column.sql");
const MIGRATION_006: &str = include_str!("../../migrations/006_password_reset_tokens.sql");
const MIGRATION_007: &str = include_str!("../../migrations/007_mod_requests.sql");

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

    if current_version < 3 {
        conn.execute_batch(MIGRATION_003)?;
        conn.pragma_update(None, "user_version", 3)?;
    }

    if current_version < 4 {
        conn.execute_batch(MIGRATION_004)?;
        conn.pragma_update(None, "user_version", 4)?;
    }

    if current_version < 5 {
        conn.execute_batch(MIGRATION_005)?;
        conn.pragma_update(None, "user_version", 5)?;
    }

    if current_version < 6 {
        conn.execute_batch(MIGRATION_006)?;
        conn.pragma_update(None, "user_version", 6)?;
    }

    if current_version < 7 {
        conn.execute_batch(MIGRATION_007)?;
        conn.pragma_update(None, "user_version", 7)?;
    }

    Ok(())
}
