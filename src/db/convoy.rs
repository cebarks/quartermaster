use rusqlite::{params, OptionalExtension};

use super::Database;

// ponytail: dead_code allowed for incremental implementation (used in later tasks)
#[allow(dead_code)]
#[derive(Debug, Clone)]
pub struct SyncReportSummary {
    pub aid: String,
    pub username: Option<String>,
    pub result: String,
    pub client_version: Option<String>,
    pub mod_count: usize,
    pub last_sync: String,
}

#[allow(dead_code)]
#[derive(Debug, Clone)]
pub struct SyncActivity {
    pub created_at: String,
    pub source: String,
    pub event_type: String,
    pub detail: String,
}

impl Database {
    #[allow(dead_code)]
    pub fn insert_sync_event(
        &self,
        event_type: &str,
        ip: Option<&str>,
        mod_ids: Option<&str>,
        bytes_served: Option<i64>,
    ) -> rusqlite::Result<()> {
        self.conn.execute(
            "INSERT INTO convoy_sync_events (event_type, ip, mod_ids, bytes_served) VALUES (?1, ?2, ?3, ?4)",
            params![event_type, ip, mod_ids, bytes_served],
        )?;
        Ok(())
    }

    #[allow(dead_code)]
    pub fn insert_sync_report(
        &self,
        aid: &str,
        result: &str,
        mods_snapshot: Option<&str>,
        client_version: Option<&str>,
        error: Option<&str>,
        ip: Option<&str>,
    ) -> rusqlite::Result<i64> {
        let user_id: Option<i64> = self
            .conn
            .query_row(
                "SELECT id FROM users WHERE spt_profile_id = ?1",
                params![aid],
                |row| row.get(0),
            )
            .optional()?;

        self.conn.execute(
            "INSERT INTO convoy_sync_reports (user_id, aid, result, mods_snapshot, client_version, error, ip)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
            params![user_id, aid, result, mods_snapshot, client_version, error, ip],
        )?;
        Ok(self.conn.last_insert_rowid())
    }

    #[allow(dead_code)]
    pub fn get_latest_sync_reports(&self) -> rusqlite::Result<Vec<SyncReportSummary>> {
        let mut stmt = self.conn.prepare(
            "SELECT r.aid, u.username, r.result, r.client_version, r.mods_snapshot, r.created_at
             FROM convoy_sync_reports r
             LEFT JOIN users u ON r.user_id = u.id
             WHERE r.id IN (
                 SELECT MAX(id) FROM convoy_sync_reports GROUP BY aid
             )
             ORDER BY r.created_at DESC",
        )?;
        let rows = stmt.query_map([], |row| {
            let snapshot: Option<String> = row.get(4)?;
            let mod_count = snapshot
                .as_deref()
                .and_then(|s| serde_json::from_str::<Vec<serde_json::Value>>(s).ok())
                .map(|v| v.len())
                .unwrap_or(0);
            Ok(SyncReportSummary {
                aid: row.get(0)?,
                username: row.get(1)?,
                result: row.get(2)?,
                client_version: row.get(3)?,
                mod_count,
                last_sync: row.get(5)?,
            })
        })?;
        rows.collect()
    }

    #[allow(dead_code)]
    pub fn get_recent_sync_activity(&self, limit: i64) -> rusqlite::Result<Vec<SyncActivity>> {
        let mut stmt = self.conn.prepare(
            "SELECT created_at, source, event_type, detail FROM (
                 SELECT created_at, 'event' AS source, event_type,
                        COALESCE('IP: ' || ip, '') AS detail
                 FROM convoy_sync_events
                 UNION ALL
                 SELECT r.created_at, 'report' AS source, r.result AS event_type,
                        COALESCE(u.username, r.aid) AS detail
                 FROM convoy_sync_reports r
                 LEFT JOIN users u ON r.user_id = u.id
             )
             ORDER BY created_at DESC
             LIMIT ?1",
        )?;
        let rows = stmt.query_map(params![limit], |row| {
            Ok(SyncActivity {
                created_at: row.get(0)?,
                source: row.get(1)?,
                event_type: row.get(2)?,
                detail: row.get(3)?,
            })
        })?;
        rows.collect()
    }

    #[allow(dead_code)]
    pub fn cleanup_old_sync_data(&self, days: i64) -> rusqlite::Result<(usize, usize)> {
        let events = self.conn.execute(
            "DELETE FROM convoy_sync_events WHERE created_at < datetime('now', ?1)",
            params![format!("-{days} days")],
        )?;
        let reports = self.conn.execute(
            "DELETE FROM convoy_sync_reports WHERE created_at < datetime('now', ?1)",
            params![format!("-{days} days")],
        )?;
        Ok((events, reports))
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;

    fn test_db() -> Database {
        Database::open_in_memory().unwrap()
    }

    #[test]
    fn insert_and_query_sync_event() {
        let db = test_db();
        db.insert_sync_event("catalog_fetch", Some("1.2.3.4"), None, None)
            .unwrap();
        db.insert_sync_event("download", Some("1.2.3.4"), Some("[1,2]"), Some(1024))
            .unwrap();

        let activity = db.get_recent_sync_activity(10).unwrap();
        assert_eq!(activity.len(), 2);
        assert_eq!(activity[0].source, "event");
    }

    #[test]
    fn insert_sync_report_links_user() {
        let db = test_db();
        db.insert_user("alice", Some("aid-123"), Some("hash"), "player", false)
            .unwrap();

        let id = db
            .insert_sync_report(
                "aid-123",
                "up_to_date",
                Some(r#"[{"id":1,"version":"1.0"}]"#),
                Some("0.1.0"),
                None,
                Some("5.6.7.8"),
            )
            .unwrap();
        assert!(id > 0);

        let reports = db.get_latest_sync_reports().unwrap();
        assert_eq!(reports.len(), 1);
        assert_eq!(reports[0].username.as_deref(), Some("alice"));
        assert_eq!(reports[0].result, "up_to_date");
        assert_eq!(reports[0].mod_count, 1);
    }

    #[test]
    fn insert_sync_report_unknown_aid() {
        let db = test_db();
        let id = db
            .insert_sync_report("unknown-aid", "failed", None, None, Some("timeout"), None)
            .unwrap();
        assert!(id > 0);

        let reports = db.get_latest_sync_reports().unwrap();
        assert_eq!(reports.len(), 1);
        assert!(reports[0].username.is_none());
    }

    #[test]
    fn latest_reports_deduplicates_by_aid() {
        let db = test_db();
        db.insert_sync_report("aid-1", "updated", None, Some("0.1.0"), None, None)
            .unwrap();
        db.insert_sync_report("aid-1", "up_to_date", None, Some("0.1.0"), None, None)
            .unwrap();

        let reports = db.get_latest_sync_reports().unwrap();
        assert_eq!(reports.len(), 1);
        assert_eq!(reports[0].result, "up_to_date");
    }

    #[test]
    fn recent_activity_merges_events_and_reports() {
        let db = test_db();
        db.insert_sync_event("catalog_fetch", Some("1.2.3.4"), None, None)
            .unwrap();
        db.insert_sync_report("aid-1", "updated", None, None, None, None)
            .unwrap();

        let activity = db.get_recent_sync_activity(10).unwrap();
        assert_eq!(activity.len(), 2);
        let sources: Vec<&str> = activity.iter().map(|a| a.source.as_str()).collect();
        assert!(sources.contains(&"event"));
        assert!(sources.contains(&"report"));
    }

    #[test]
    fn cleanup_removes_nothing_when_fresh() {
        let db = test_db();
        db.insert_sync_event("catalog_fetch", None, None, None)
            .unwrap();
        db.insert_sync_report("aid-1", "updated", None, None, None, None)
            .unwrap();

        let (events, reports) = db.cleanup_old_sync_data(30).unwrap();
        assert_eq!(events, 0);
        assert_eq!(reports, 0);

        assert_eq!(db.get_recent_sync_activity(10).unwrap().len(), 2);
    }
}
