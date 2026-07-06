use crate::db::Database;
use crate::fika::stats::SessionStats;

#[derive(Debug, Clone)]
pub struct HeadlessSessionRow {
    pub completed_at: String,
    pub sent_packets: i64,
    pub sent_data_bytes: i64,
    pub received_packets: i64,
    pub received_data_bytes: i64,
    pub packet_loss_percent: f64,
    pub time_in_raid_seconds: i64,
}

impl Database {
    pub fn get_recent_session_stats(
        &self,
        client_index: u32,
        limit: u32,
    ) -> rusqlite::Result<Vec<HeadlessSessionRow>> {
        let mut stmt = self.conn.prepare(
            "SELECT completed_at, sent_packets, sent_data_bytes, received_packets,
                    received_data_bytes, packet_loss_percent, time_in_raid_seconds
             FROM headless_session_stats
             WHERE client_index = ?1
             ORDER BY completed_at DESC
             LIMIT ?2",
        )?;

        let rows = stmt.query_map(rusqlite::params![client_index, limit], |row| {
            Ok(HeadlessSessionRow {
                completed_at: row.get(0)?,
                sent_packets: row.get(1)?,
                sent_data_bytes: row.get(2)?,
                received_packets: row.get(3)?,
                received_data_bytes: row.get(4)?,
                packet_loss_percent: row.get(5)?,
                time_in_raid_seconds: row.get(6)?,
            })
        })?;

        rows.collect()
    }

    pub fn insert_session_stats(
        &self,
        stats: &SessionStats,
        client_index: u32,
        profile_id: &str,
    ) -> rusqlite::Result<()> {
        self.conn.execute(
            "INSERT INTO headless_session_stats (client_index, profile_id, sent_packets, sent_data_bytes, received_packets, received_data_bytes, packet_loss_percent, time_in_raid_seconds) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
            rusqlite::params![
                client_index,
                profile_id,
                stats.sent_packets as i64,
                stats.sent_data_bytes as i64,
                stats.received_packets as i64,
                stats.received_data_bytes as i64,
                stats.packet_loss_percent,
                stats.time_in_raid_seconds,
            ],
        )?;
        Ok(())
    }
}
