use crate::logging::LogEntry;
use rusqlite::params;

#[allow(dead_code)] // Used in Task 5 (log API endpoints)
#[derive(Debug, Clone, serde::Serialize)]
pub struct StoredLogEntry {
    pub id: i64,
    pub timestamp: String,
    pub level: String,
    pub target: String,
    pub message: String,
    pub fields: Option<String>,
}

#[allow(dead_code)] // Used in Task 5 (log API endpoints)
pub struct LogQuery {
    pub level: Option<String>,
    pub target: Option<String>,
    pub search: Option<String>,
    pub before: Option<i64>,
    pub limit: usize,
}

impl super::Database {
    pub fn insert_log_batch(&self, entries: &[LogEntry]) -> rusqlite::Result<()> {
        let tx = self.conn.unchecked_transaction()?;
        {
            let mut stmt = tx.prepare_cached(
                "INSERT INTO log_entries (timestamp, level, target, message, fields) VALUES (?1, ?2, ?3, ?4, ?5)"
            )?;
            for entry in entries {
                let fields_json = if entry.fields.is_empty() {
                    None
                } else {
                    serde_json::to_string(&entry.fields).ok()
                };
                stmt.execute(params![
                    entry.timestamp.format("%Y-%m-%d %H:%M:%S").to_string(),
                    entry.level,
                    entry.target,
                    entry.message,
                    fields_json,
                ])?;
            }
        }
        tx.commit()
    }

    #[allow(dead_code)] // Used in Task 5 (log API endpoints)
    pub fn query_logs(&self, query: &LogQuery) -> rusqlite::Result<Vec<StoredLogEntry>> {
        let mut sql = String::from(
            "SELECT id, timestamp, level, target, message, fields FROM log_entries WHERE 1=1",
        );
        let mut param_values: Vec<Box<dyn rusqlite::types::ToSql>> = Vec::new();

        if let Some(ref level) = query.level {
            sql.push_str(" AND level = ?");
            param_values.push(Box::new(level.clone()));
        }
        if let Some(ref target) = query.target {
            sql.push_str(" AND target LIKE ?");
            param_values.push(Box::new(format!("{target}%")));
        }
        if let Some(ref search) = query.search {
            sql.push_str(" AND message LIKE ?");
            param_values.push(Box::new(format!("%{search}%")));
        }
        if let Some(before) = query.before {
            sql.push_str(" AND id < ?");
            param_values.push(Box::new(before));
        }

        sql.push_str(" ORDER BY id DESC LIMIT ?");
        param_values.push(Box::new(query.limit as i64));

        let params_refs: Vec<&dyn rusqlite::types::ToSql> =
            param_values.iter().map(|p| p.as_ref()).collect();
        let mut stmt = self.conn.prepare(&sql)?;
        let rows = stmt.query_map(params_refs.as_slice(), |row| {
            Ok(StoredLogEntry {
                id: row.get(0)?,
                timestamp: row.get(1)?,
                level: row.get(2)?,
                target: row.get(3)?,
                message: row.get(4)?,
                fields: row.get(5)?,
            })
        })?;

        rows.collect()
    }

    #[allow(dead_code)] // Used in Task 5 (log API endpoints)
    pub fn log_counts_by_level(&self) -> rusqlite::Result<std::collections::HashMap<String, i64>> {
        let mut stmt = self
            .conn
            .prepare("SELECT level, COUNT(*) FROM log_entries GROUP BY level")?;
        let rows = stmt.query_map([], |row| {
            Ok((row.get::<_, String>(0)?, row.get::<_, i64>(1)?))
        })?;
        rows.collect::<rusqlite::Result<std::collections::HashMap<_, _>>>()
    }

    pub fn prune_logs_batch(
        &self,
        max_age_days: u64,
        max_entries: u64,
        batch_size: u64,
    ) -> rusqlite::Result<u64> {
        let mut total_deleted = 0u64;

        // Prune by age
        loop {
            let deleted = self.conn.execute(
                "DELETE FROM log_entries WHERE id IN (
                    SELECT id FROM log_entries
                    WHERE timestamp < datetime('now', ?1)
                    LIMIT ?2
                )",
                params![format!("-{max_age_days} days"), batch_size as i64],
            )?;
            total_deleted += deleted as u64;
            if (deleted as u64) < batch_size {
                break;
            }
        }

        // Prune by count
        loop {
            let count: i64 =
                self.conn
                    .query_row("SELECT COUNT(*) FROM log_entries", [], |row| row.get(0))?;
            if count as u64 <= max_entries {
                break;
            }
            let to_delete = ((count as u64) - max_entries).min(batch_size);
            let deleted = self.conn.execute(
                "DELETE FROM log_entries WHERE id IN (
                    SELECT id FROM log_entries ORDER BY id ASC LIMIT ?1
                )",
                params![to_delete as i64],
            )?;
            total_deleted += deleted as u64;
            if deleted == 0 {
                break;
            }
        }

        Ok(total_deleted)
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;
    use crate::db::Database;
    use chrono::Utc;
    use std::collections::HashMap;

    fn test_entry(level: &str, msg: &str) -> LogEntry {
        LogEntry {
            timestamp: Utc::now(),
            level: level.to_string(),
            target: "test::target".to_string(),
            message: msg.to_string(),
            fields: HashMap::new(),
        }
    }

    #[test]
    fn insert_and_query_logs() {
        let db = Database::open_in_memory().unwrap();
        let entries = vec![
            test_entry("INFO", "first"),
            test_entry("WARN", "second"),
            test_entry("ERROR", "third"),
        ];
        db.insert_log_batch(&entries).unwrap();

        let results = db
            .query_logs(&LogQuery {
                level: None,
                target: None,
                search: None,
                before: None,
                limit: 10,
            })
            .unwrap();
        assert_eq!(results.len(), 3);
        assert_eq!(results[0].message, "third"); // newest first
    }

    #[test]
    fn query_logs_filters_by_level() {
        let db = Database::open_in_memory().unwrap();
        db.insert_log_batch(&vec![
            test_entry("INFO", "a"),
            test_entry("WARN", "b"),
            test_entry("INFO", "c"),
        ])
        .unwrap();

        let results = db
            .query_logs(&LogQuery {
                level: Some("WARN".to_string()),
                target: None,
                search: None,
                before: None,
                limit: 10,
            })
            .unwrap();
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].message, "b");
    }

    #[test]
    fn query_logs_cursor_pagination() {
        let db = Database::open_in_memory().unwrap();
        for i in 0..5 {
            db.insert_log_batch(&vec![test_entry("INFO", &format!("msg{i}"))])
                .unwrap();
        }

        let page1 = db
            .query_logs(&LogQuery {
                level: None,
                target: None,
                search: None,
                before: None,
                limit: 2,
            })
            .unwrap();
        assert_eq!(page1.len(), 2);
        assert_eq!(page1[0].message, "msg4");

        let page2 = db
            .query_logs(&LogQuery {
                level: None,
                target: None,
                search: None,
                before: Some(page1[1].id),
                limit: 2,
            })
            .unwrap();
        assert_eq!(page2.len(), 2);
        assert_eq!(page2[0].message, "msg2");
    }

    #[test]
    fn prune_logs_by_count() {
        let db = Database::open_in_memory().unwrap();
        for i in 0..20 {
            db.insert_log_batch(&vec![test_entry("INFO", &format!("msg{i}"))])
                .unwrap();
        }
        let deleted = db.prune_logs_batch(365, 10, 100).unwrap();
        assert_eq!(deleted, 10);

        let remaining = db
            .query_logs(&LogQuery {
                level: None,
                target: None,
                search: None,
                before: None,
                limit: 100,
            })
            .unwrap();
        assert_eq!(remaining.len(), 10);
        assert_eq!(remaining[0].message, "msg19"); // newest kept
    }
}
