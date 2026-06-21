use rusqlite::Connection;

const MIGRATION_001: &str = include_str!("../../migrations/001_initial.sql");
const MIGRATION_002: &str = include_str!("../../migrations/002_cascade_depends_on.sql");
const MIGRATION_003: &str = include_str!("../../migrations/003_file_mod_id_index.sql");
const MIGRATION_004: &str = include_str!("../../migrations/004_file_source.sql");
const MIGRATION_005: &str = include_str!("../../migrations/005_add_disabled_column.sql");
const MIGRATION_006: &str = include_str!("../../migrations/006_password_reset_tokens.sql");
const MIGRATION_007: &str = include_str!("../../migrations/007_mod_requests.sql");
const MIGRATION_008: &str = include_str!("../../migrations/008_nullable_profile_id.sql");
const MIGRATION_009: &str = include_str!("../../migrations/009_add_mod_disabled.sql");
const MIGRATION_010: &str = include_str!("../../migrations/010_raid_tracking.sql");
const MIGRATION_011: &str = include_str!("../../migrations/011_stash_public.sql");
const MIGRATION_012: &str = include_str!("../../migrations/012_raid_snapshots.sql");

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

    if current_version < 8 {
        conn.execute_batch(MIGRATION_008)?;
        conn.pragma_update(None, "user_version", 8)?;
    }

    if current_version < 9 {
        conn.execute_batch(MIGRATION_009)?;
        conn.pragma_update(None, "user_version", 9)?;
    }

    if current_version < 10 {
        conn.execute_batch(MIGRATION_010)?;
        conn.pragma_update(None, "user_version", 10)?;
    }

    if current_version < 11 {
        conn.execute_batch(MIGRATION_011)?;
        conn.pragma_update(None, "user_version", 11)?;
    }

    if current_version < 12 {
        conn.execute_batch(MIGRATION_012)?;
        conn.pragma_update(None, "user_version", 12)?;
    }

    Ok(())
}
