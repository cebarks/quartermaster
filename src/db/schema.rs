use rusqlite::Connection;

/// Ordered list of migration SQL scripts. Each entry corresponds to a
/// `user_version` bump: index 0 → version 1, index 1 → version 2, etc.
/// To add a new migration, append a new `include_str!` entry here.
const MIGRATIONS: &[&str] = &[
    include_str!("../../migrations/001_initial.sql"),
    include_str!("../../migrations/002_cascade_depends_on.sql"),
    include_str!("../../migrations/003_file_mod_id_index.sql"),
    include_str!("../../migrations/004_file_source.sql"),
    include_str!("../../migrations/005_add_disabled_column.sql"),
    include_str!("../../migrations/006_password_reset_tokens.sql"),
    include_str!("../../migrations/007_mod_requests.sql"),
    include_str!("../../migrations/008_nullable_profile_id.sql"),
    include_str!("../../migrations/009_add_mod_disabled.sql"),
    include_str!("../../migrations/010_raid_tracking.sql"),
    include_str!("../../migrations/011_stash_public.sql"),
    include_str!("../../migrations/012_raid_snapshots.sql"),
    include_str!("../../migrations/013_cascade_mod_requests.sql"),
];

pub fn run_migrations(conn: &Connection) -> rusqlite::Result<()> {
    let current_version: i32 = conn.pragma_query_value(None, "user_version", |row| row.get(0))?;

    for (i, sql) in MIGRATIONS.iter().enumerate() {
        let target_version = (i + 1) as i32;
        if current_version < target_version {
            conn.execute_batch(sql)?;
            conn.pragma_update(None, "user_version", target_version)?;
        }
    }

    Ok(())
}
