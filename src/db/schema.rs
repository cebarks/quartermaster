use rusqlite::Connection;

/// Ordered list of migration SQL scripts. Each entry corresponds to a
/// `user_version` bump: index 0 → version 1, index 1 → version 2, etc.
/// To add a new migration, append a new `include_str!` entry here.
const MIGRATIONS: &[&str] = &[
    include_str!("../../migrations/001_initial.sql"),
    include_str!("../../migrations/002_pending_updates.sql"),
    include_str!("../../migrations/003_log_entries.sql"),
    include_str!("../../migrations/004_invite_codes_cascade.sql"),
];

pub fn run_migrations(conn: &Connection) -> rusqlite::Result<()> {
    let current_version: i32 = conn.pragma_query_value(None, "user_version", |row| row.get(0))?;

    for (i, sql) in MIGRATIONS.iter().enumerate() {
        let target_version = (i + 1) as i32;
        if current_version < target_version {
            let tx = conn.unchecked_transaction()?;
            tx.execute_batch(sql)?;
            tx.pragma_update(None, "user_version", target_version)?;
            tx.commit()?;
        }
    }

    Ok(())
}
