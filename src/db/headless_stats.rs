use crate::db::Database;
use crate::fika::stats::SessionStats;

impl Database {
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
