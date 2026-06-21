use rusqlite::{params, OptionalExtension};

use flate2::read::ZlibDecoder;
use flate2::write::ZlibEncoder;
use flate2::Compression;
use std::io::{Read, Write};

use super::Database;

#[derive(Debug, Clone)]
#[allow(dead_code)] // SQL row model — fields populated by query results
pub struct Raid {
    pub id: i64,
    pub user_id: i64,
    pub spt_profile_id: String,
    pub server_id: Option<String>,
    pub player_side: String,
    pub faction: Option<String>,
    pub map: String,
    pub time_variant: Option<String>,
    pub started_at: String,
    pub ended_at: Option<String>,
    pub play_time_seconds: Option<i64>,
    pub exit_status: Option<String>,
    pub exit_name: Option<String>,
    pub killer_id: Option<String>,
    pub killer_aid: Option<String>,
    pub xp_before: Option<i64>,
    pub xp_after: Option<i64>,
    pub level_before: Option<i64>,
    pub level_after: Option<i64>,
    pub victim_count_before: Option<i64>,
}

#[derive(Debug, Clone)]
#[allow(dead_code)] // SQL row model
pub struct RaidKill {
    pub id: i64,
    pub raid_id: i64,
    pub victim_name: Option<String>,
    pub victim_side: Option<String>,
    pub victim_role: Option<String>,
    pub weapon: Option<String>,
    pub distance: Option<f64>,
    pub body_part: Option<String>,
    pub kill_time: Option<String>,
}

#[derive(Debug, Clone)]
#[allow(dead_code)] // Used by Tasks 2-4 for raid event processing
pub struct NewRaidKill {
    pub victim_name: Option<String>,
    pub victim_side: Option<String>,
    pub victim_role: Option<String>,
    pub weapon: Option<String>,
    pub distance: Option<f64>,
    pub body_part: Option<String>,
    pub kill_time: Option<String>,
}

#[derive(Debug, Clone)]
#[allow(dead_code)] // Used by Task 3 (web UI)
pub struct UserRaidStats {
    pub total_raids: i64,
    pub pmc_raids: i64,
    pub scav_raids: i64,
    pub pmc_survival_rate: f64,
    pub scav_survival_rate: f64,
    pub total_kills: i64,
    pub pmc_kills: i64,
    pub scav_kills: i64,
    pub total_deaths: i64,
    pub avg_raid_duration: f64,
    pub favorite_map: Option<String>,
    pub pmc_xp_gained: i64,
    pub scav_xp_gained: i64,
}

#[derive(Debug, Clone)]
#[allow(dead_code)] // Used by Task 3 (web UI)
pub struct ServerRaidStats {
    pub total_raids: i64,
    pub unique_players: i64,
    pub overall_survival_rate: f64,
    pub map_counts: Vec<(String, i64)>,
    pub top_killers: Vec<(String, i64)>,
    pub recent_raids: Vec<(Raid, String)>,
}

#[derive(Debug, Clone)]
pub struct LeaderboardEntry {
    pub username: String,
    pub total_raids: i64,
    pub total_kills: i64,
    pub total_deaths: i64,
    pub kd_ratio: f64,
    pub survival_rate: f64,
    pub headshot_count: i64,
    pub headshot_ratio: f64,
    pub longest_kill: f64,
    pub favorite_weapon: Option<String>,
    pub favorite_map: Option<String>,
    pub favorite_extract: Option<String>,
}

pub fn compress_snapshot(json: &[u8]) -> Result<Vec<u8>, std::io::Error> {
    let mut encoder = ZlibEncoder::new(Vec::new(), Compression::default());
    encoder.write_all(json)?;
    encoder.finish()
}

#[allow(dead_code)] // Used by Task 5 (raid detail handler)
pub fn decompress_snapshot(data: &[u8]) -> Result<Vec<u8>, std::io::Error> {
    let mut decoder = ZlibDecoder::new(data);
    let mut buf = Vec::new();
    decoder.read_to_end(&mut buf)?;
    Ok(buf)
}

impl Database {
    #[allow(dead_code)] // Used by Task 2 (raid start event)
    #[allow(clippy::too_many_arguments)] // All parameters needed for raid creation
    pub fn insert_raid(
        &self,
        user_id: i64,
        spt_profile_id: &str,
        server_id: Option<&str>,
        player_side: &str,
        faction: Option<&str>,
        map: &str,
        time_variant: Option<&str>,
        started_at: &str,
        xp_before: Option<i64>,
        level_before: Option<i64>,
        victim_count_before: Option<i64>,
    ) -> rusqlite::Result<i64> {
        self.conn.execute(
            "INSERT INTO raids (user_id, spt_profile_id, server_id, player_side, faction, map, time_variant, started_at, xp_before, level_before, victim_count_before)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11)",
            params![
                user_id,
                spt_profile_id,
                server_id,
                player_side,
                faction,
                map,
                time_variant,
                started_at,
                xp_before,
                level_before,
                victim_count_before
            ],
        )?;
        Ok(self.conn.last_insert_rowid())
    }

    #[allow(dead_code)] // Used by Task 2 (raid start event to close previous open raids)
    pub fn close_orphaned_raids(&self, spt_profile_id: &str) -> rusqlite::Result<usize> {
        self.conn.execute(
            "UPDATE raids SET exit_status = 'Unknown', ended_at = datetime('now')
             WHERE spt_profile_id = ?1 AND ended_at IS NULL",
            params![spt_profile_id],
        )
    }

    #[allow(dead_code)] // Used by Task 2 (cleanup job for orphaned raids)
    pub fn close_stale_raids(&self, max_age_hours: i64) -> rusqlite::Result<usize> {
        self.conn.execute(
            "UPDATE raids SET exit_status = 'Unknown', ended_at = datetime('now')
             WHERE ended_at IS NULL AND started_at < datetime('now', '-' || ?1 || ' hours')",
            params![max_age_hours],
        )
    }

    #[allow(dead_code)] // Used by Task 2 (raid end event)
    #[allow(clippy::too_many_arguments)] // All parameters needed for raid completion
    pub fn finish_raid(
        &self,
        raid_id: i64,
        ended_at: &str,
        play_time_seconds: Option<i64>,
        exit_status: &str,
        exit_name: Option<&str>,
        killer_id: Option<&str>,
        killer_aid: Option<&str>,
        xp_after: Option<i64>,
        level_after: Option<i64>,
    ) -> rusqlite::Result<usize> {
        self.conn.execute(
            "UPDATE raids SET ended_at = ?1, play_time_seconds = ?2, exit_status = ?3, exit_name = ?4, killer_id = ?5, killer_aid = ?6, xp_after = ?7, level_after = ?8
             WHERE id = ?9",
            params![
                ended_at,
                play_time_seconds,
                exit_status,
                exit_name,
                killer_id,
                killer_aid,
                xp_after,
                level_after,
                raid_id
            ],
        )
    }

    #[allow(dead_code)] // Used by Task 2 (raid end event)
    pub fn insert_raid_kills(&self, raid_id: i64, kills: &[NewRaidKill]) -> rusqlite::Result<()> {
        if kills.is_empty() {
            return Ok(());
        }
        let tx = self.begin_transaction()?;
        for kill in kills {
            self.conn.execute(
                "INSERT INTO raid_kills (raid_id, victim_name, victim_side, victim_role, weapon, distance, body_part, kill_time)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
                params![
                    raid_id,
                    kill.victim_name,
                    kill.victim_side,
                    kill.victim_role,
                    kill.weapon,
                    kill.distance,
                    kill.body_part,
                    kill.kill_time
                ],
            )?;
        }
        tx.commit()
    }

    #[allow(clippy::too_many_arguments)]
    pub fn finish_raid_with_kills(
        &self,
        raid_id: i64,
        ended_at: &str,
        play_time_seconds: Option<i64>,
        exit_status: &str,
        exit_name: Option<&str>,
        killer_id: Option<&str>,
        killer_aid: Option<&str>,
        xp_after: Option<i64>,
        level_after: Option<i64>,
        kills: &[NewRaidKill],
    ) -> rusqlite::Result<()> {
        let tx = self.begin_transaction()?;
        self.finish_raid(
            raid_id,
            ended_at,
            play_time_seconds,
            exit_status,
            exit_name,
            killer_id,
            killer_aid,
            xp_after,
            level_after,
        )?;
        for kill in kills {
            self.conn.execute(
                "INSERT INTO raid_kills (raid_id, victim_name, victim_side, victim_role, weapon, distance, body_part, kill_time)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
                params![
                    raid_id,
                    kill.victim_name,
                    kill.victim_side,
                    kill.victim_role,
                    kill.weapon,
                    kill.distance,
                    kill.body_part,
                    kill.kill_time
                ],
            )?;
        }
        tx.commit()
    }

    #[allow(dead_code)] // Used by Task 3 (web UI)
    pub fn get_raids_for_user(
        &self,
        user_id: i64,
        limit: i64,
        offset: i64,
    ) -> rusqlite::Result<Vec<Raid>> {
        let mut stmt = self.conn.prepare(
            "SELECT id, user_id, spt_profile_id, server_id, player_side, faction, map, time_variant, started_at, ended_at, play_time_seconds, exit_status, exit_name, killer_id, killer_aid, xp_before, xp_after, level_before, level_after, victim_count_before
             FROM raids WHERE user_id = ?1 ORDER BY started_at DESC LIMIT ?2 OFFSET ?3",
        )?;
        let rows = stmt.query_map(params![user_id, limit, offset], row_to_raid)?;
        rows.collect()
    }

    #[allow(dead_code)] // Used by Task 3 (web UI)
    pub fn get_raid_with_kills(
        &self,
        raid_id: i64,
    ) -> rusqlite::Result<Option<(Raid, Vec<RaidKill>)>> {
        let raid = self
            .conn
            .query_row(
                "SELECT id, user_id, spt_profile_id, server_id, player_side, faction, map, time_variant, started_at, ended_at, play_time_seconds, exit_status, exit_name, killer_id, killer_aid, xp_before, xp_after, level_before, level_after, victim_count_before
                 FROM raids WHERE id = ?1",
                params![raid_id],
                row_to_raid,
            )
            .optional()?;

        if let Some(raid) = raid {
            let mut stmt = self.conn.prepare(
                "SELECT id, raid_id, victim_name, victim_side, victim_role, weapon, distance, body_part, kill_time
                 FROM raid_kills WHERE raid_id = ?1 ORDER BY kill_time",
            )?;
            let kills = stmt
                .query_map(params![raid_id], row_to_raid_kill)?
                .collect::<rusqlite::Result<Vec<_>>>()?;
            Ok(Some((raid, kills)))
        } else {
            Ok(None)
        }
    }

    #[allow(dead_code)] // Used by Task 3 (web UI for squad raids)
    pub fn get_raid_group(&self, server_id: &str) -> rusqlite::Result<Vec<(Raid, String)>> {
        let mut stmt = self.conn.prepare(
            "SELECT r.id, r.user_id, r.spt_profile_id, r.server_id, r.player_side, r.faction, r.map, r.time_variant, r.started_at, r.ended_at, r.play_time_seconds, r.exit_status, r.exit_name, r.killer_id, r.killer_aid, r.xp_before, r.xp_after, r.level_before, r.level_after, r.victim_count_before, u.username
             FROM raids r
             JOIN users u ON r.user_id = u.id
             WHERE r.server_id = ?1
             ORDER BY r.started_at",
        )?;
        let rows = stmt.query_map(params![server_id], |row| {
            let raid = row_to_raid(row)?;
            let username: String = row.get(20)?;
            Ok((raid, username))
        })?;
        rows.collect()
    }

    #[allow(dead_code)] // Used by Task 3 (web UI dashboard)
    pub fn get_active_raids(&self) -> rusqlite::Result<Vec<(Raid, String)>> {
        let mut stmt = self.conn.prepare(
            "SELECT r.id, r.user_id, r.spt_profile_id, r.server_id, r.player_side, r.faction, r.map, r.time_variant, r.started_at, r.ended_at, r.play_time_seconds, r.exit_status, r.exit_name, r.killer_id, r.killer_aid, r.xp_before, r.xp_after, r.level_before, r.level_after, r.victim_count_before, u.username
             FROM raids r
             JOIN users u ON r.user_id = u.id
             WHERE r.ended_at IS NULL
             ORDER BY r.started_at DESC",
        )?;
        let rows = stmt.query_map([], |row| {
            let raid = row_to_raid(row)?;
            let username: String = row.get(20)?;
            Ok((raid, username))
        })?;
        rows.collect()
    }

    #[allow(dead_code)] // Used by Task 3 (web UI)
    pub fn get_user_raid_stats(&self, user_id: i64) -> rusqlite::Result<UserRaidStats> {
        let total_raids: i64 = self.conn.query_row(
            "SELECT COUNT(*) FROM raids WHERE user_id = ?1",
            params![user_id],
            |row| row.get(0),
        )?;

        let pmc_raids: i64 = self.conn.query_row(
            "SELECT COUNT(*) FROM raids WHERE user_id = ?1 AND player_side = 'Pmc'",
            params![user_id],
            |row| row.get(0),
        )?;

        let scav_raids: i64 = self.conn.query_row(
            "SELECT COUNT(*) FROM raids WHERE user_id = ?1 AND player_side = 'Savage'",
            params![user_id],
            |row| row.get(0),
        )?;

        let pmc_survived: i64 = self.conn.query_row(
            "SELECT COUNT(*) FROM raids WHERE user_id = ?1 AND player_side = 'Pmc' AND exit_status = 'Survived'",
            params![user_id],
            |row| row.get(0),
        )?;

        let pmc_survival_rate = if pmc_raids > 0 {
            (pmc_survived as f64 / pmc_raids as f64) * 100.0
        } else {
            0.0
        };

        let scav_survived: i64 = self.conn.query_row(
            "SELECT COUNT(*) FROM raids WHERE user_id = ?1 AND player_side = 'Savage' AND exit_status = 'Survived'",
            params![user_id],
            |row| row.get(0),
        )?;

        let scav_survival_rate = if scav_raids > 0 {
            (scav_survived as f64 / scav_raids as f64) * 100.0
        } else {
            0.0
        };

        let total_kills: i64 = self.conn.query_row(
            "SELECT COUNT(*) FROM raid_kills rk
             JOIN raids r ON rk.raid_id = r.id
             WHERE r.user_id = ?1",
            params![user_id],
            |row| row.get(0),
        )?;

        let pmc_kills: i64 = self.conn.query_row(
            "SELECT COUNT(*) FROM raid_kills rk
             JOIN raids r ON rk.raid_id = r.id
             WHERE r.user_id = ?1 AND rk.victim_side IN ('Usec', 'Bear')",
            params![user_id],
            |row| row.get(0),
        )?;

        let scav_kills: i64 = self.conn.query_row(
            "SELECT COUNT(*) FROM raid_kills rk
             JOIN raids r ON rk.raid_id = r.id
             WHERE r.user_id = ?1 AND rk.victim_side = 'Savage'",
            params![user_id],
            |row| row.get(0),
        )?;

        let total_deaths: i64 = self.conn.query_row(
            "SELECT COUNT(*) FROM raids WHERE user_id = ?1 AND exit_status = 'Killed'",
            params![user_id],
            |row| row.get(0),
        )?;

        let avg_raid_duration: f64 = self
            .conn
            .query_row(
                "SELECT AVG(play_time_seconds) FROM raids WHERE user_id = ?1 AND play_time_seconds IS NOT NULL",
                params![user_id],
                |row| row.get(0),
            )
            .unwrap_or(0.0);

        let favorite_map: Option<String> = self
            .conn
            .query_row(
                "SELECT map FROM raids WHERE user_id = ?1 GROUP BY map ORDER BY COUNT(*) DESC LIMIT 1",
                params![user_id],
                |row| row.get(0),
            )
            .optional()?;

        let pmc_xp_gained: i64 = self.conn.query_row(
            "SELECT COALESCE(SUM(COALESCE(xp_after, 0) - COALESCE(xp_before, 0)), 0)
             FROM raids WHERE user_id = ?1 AND player_side = 'Pmc' AND xp_after IS NOT NULL AND xp_before IS NOT NULL",
            params![user_id],
            |row| row.get(0),
        )?;

        let scav_xp_gained: i64 = self.conn.query_row(
            "SELECT COALESCE(SUM(COALESCE(xp_after, 0) - COALESCE(xp_before, 0)), 0)
             FROM raids WHERE user_id = ?1 AND player_side = 'Savage' AND xp_after IS NOT NULL AND xp_before IS NOT NULL",
            params![user_id],
            |row| row.get(0),
        )?;

        Ok(UserRaidStats {
            total_raids,
            pmc_raids,
            scav_raids,
            pmc_survival_rate,
            scav_survival_rate,
            total_kills,
            pmc_kills,
            scav_kills,
            total_deaths,
            avg_raid_duration,
            favorite_map,
            pmc_xp_gained,
            scav_xp_gained,
        })
    }

    #[allow(dead_code)] // Used by Task 3 (web UI dashboard)
    pub fn get_server_raid_stats(&self) -> rusqlite::Result<ServerRaidStats> {
        let total_raids: i64 = self
            .conn
            .query_row("SELECT COUNT(*) FROM raids", [], |row| row.get(0))?;

        let unique_players: i64 =
            self.conn
                .query_row("SELECT COUNT(DISTINCT user_id) FROM raids", [], |row| {
                    row.get(0)
                })?;

        let survived_count: i64 = self.conn.query_row(
            "SELECT COUNT(*) FROM raids WHERE exit_status = 'Survived'",
            [],
            |row| row.get(0),
        )?;

        let overall_survival_rate = if total_raids > 0 {
            (survived_count as f64 / total_raids as f64) * 100.0
        } else {
            0.0
        };

        let mut map_stmt = self
            .conn
            .prepare("SELECT map, COUNT(*) as count FROM raids GROUP BY map ORDER BY count DESC")?;
        let map_counts: Vec<(String, i64)> = map_stmt
            .query_map([], |row| Ok((row.get(0)?, row.get(1)?)))?
            .collect::<rusqlite::Result<Vec<_>>>()?;

        let mut killer_stmt = self.conn.prepare(
            "SELECT u.username, COUNT(rk.id) as kill_count
             FROM raid_kills rk
             JOIN raids r ON rk.raid_id = r.id
             JOIN users u ON r.user_id = u.id
             GROUP BY u.username
             ORDER BY kill_count DESC
             LIMIT 5",
        )?;
        let top_killers: Vec<(String, i64)> = killer_stmt
            .query_map([], |row| Ok((row.get(0)?, row.get(1)?)))?
            .collect::<rusqlite::Result<Vec<_>>>()?;

        let recent_raids = self.get_recent_raids(10)?;

        Ok(ServerRaidStats {
            total_raids,
            unique_players,
            overall_survival_rate,
            map_counts,
            top_killers,
            recent_raids,
        })
    }

    #[allow(dead_code)] // Used by Task 3 (web UI) and get_server_raid_stats
    pub fn get_recent_raids(&self, limit: i64) -> rusqlite::Result<Vec<(Raid, String)>> {
        let mut stmt = self.conn.prepare(
            "SELECT r.id, r.user_id, r.spt_profile_id, r.server_id, r.player_side, r.faction, r.map, r.time_variant, r.started_at, r.ended_at, r.play_time_seconds, r.exit_status, r.exit_name, r.killer_id, r.killer_aid, r.xp_before, r.xp_after, r.level_before, r.level_after, r.victim_count_before, u.username
             FROM raids r
             JOIN users u ON r.user_id = u.id
             WHERE r.ended_at IS NOT NULL
             ORDER BY r.ended_at DESC
             LIMIT ?1",
        )?;
        let rows = stmt.query_map(params![limit], |row| {
            let raid = row_to_raid(row)?;
            let username: String = row.get(20)?;
            Ok((raid, username))
        })?;
        rows.collect()
    }

    pub fn get_leaderboard(&self, min_raids: u32) -> rusqlite::Result<Vec<LeaderboardEntry>> {
        let mut stmt = self.conn.prepare(
            "WITH raid_stats AS (
                SELECT
                    r.user_id,
                    u.username,
                    COUNT(*) AS total_raids,
                    SUM(CASE WHEN r.exit_status = 'Survived' THEN 1 ELSE 0 END) AS survived,
                    SUM(CASE WHEN r.exit_status = 'Killed' THEN 1 ELSE 0 END) AS total_deaths,
                    (SELECT r2.map FROM raids r2
                     WHERE r2.user_id = r.user_id AND r2.ended_at IS NOT NULL
                     GROUP BY r2.map ORDER BY COUNT(*) DESC, r2.map ASC LIMIT 1
                    ) AS favorite_map,
                    (SELECT r3.exit_name FROM raids r3
                     WHERE r3.user_id = r.user_id AND r3.exit_status = 'Survived'
                       AND r3.exit_name IS NOT NULL AND r3.ended_at IS NOT NULL
                     GROUP BY r3.exit_name ORDER BY COUNT(*) DESC, r3.exit_name ASC LIMIT 1
                    ) AS favorite_extract
                FROM raids r
                JOIN users u ON r.user_id = u.id
                WHERE r.ended_at IS NOT NULL
                GROUP BY r.user_id
                HAVING COUNT(*) >= ?1
            ),
            kill_stats AS (
                SELECT
                    r.user_id,
                    COUNT(rk.id) AS total_kills,
                    SUM(CASE WHEN rk.body_part = 'Head' THEN 1 ELSE 0 END) AS headshot_count,
                    MAX(rk.distance) AS longest_kill,
                    (SELECT rk2.weapon FROM raid_kills rk2
                     JOIN raids r2 ON rk2.raid_id = r2.id
                     WHERE r2.user_id = r.user_id AND rk2.weapon IS NOT NULL
                     GROUP BY rk2.weapon ORDER BY COUNT(*) DESC, rk2.weapon ASC LIMIT 1
                    ) AS favorite_weapon
                FROM raids r
                JOIN raid_kills rk ON rk.raid_id = r.id
                WHERE r.ended_at IS NOT NULL
                GROUP BY r.user_id
            )
            SELECT
                rs.username,
                rs.total_raids,
                COALESCE(ks.total_kills, 0) AS total_kills,
                rs.total_deaths,
                CASE WHEN rs.total_deaths = 0
                    THEN CAST(COALESCE(ks.total_kills, 0) AS REAL)
                    ELSE CAST(COALESCE(ks.total_kills, 0) AS REAL) / rs.total_deaths
                END AS kd_ratio,
                CASE WHEN rs.total_raids = 0 THEN 0.0
                    ELSE CAST(rs.survived AS REAL) / rs.total_raids * 100.0
                END AS survival_rate,
                COALESCE(ks.headshot_count, 0) AS headshot_count,
                CASE WHEN COALESCE(ks.total_kills, 0) = 0 THEN 0.0
                    ELSE CAST(COALESCE(ks.headshot_count, 0) AS REAL) / ks.total_kills * 100.0
                END AS headshot_ratio,
                COALESCE(ks.longest_kill, 0.0) AS longest_kill,
                ks.favorite_weapon,
                rs.favorite_map,
                rs.favorite_extract
            FROM raid_stats rs
            LEFT JOIN kill_stats ks ON rs.user_id = ks.user_id
            ORDER BY total_kills DESC",
        )?;

        let rows = stmt.query_map(params![min_raids], |row| {
            Ok(LeaderboardEntry {
                username: row.get(0)?,
                total_raids: row.get(1)?,
                total_kills: row.get(2)?,
                total_deaths: row.get(3)?,
                kd_ratio: row.get(4)?,
                survival_rate: row.get(5)?,
                headshot_count: row.get(6)?,
                headshot_ratio: row.get(7)?,
                longest_kill: row.get(8)?,
                favorite_weapon: row.get(9)?,
                favorite_map: row.get(10)?,
                favorite_extract: row.get(11)?,
            })
        })?;
        rows.collect()
    }

    #[allow(dead_code)] // Used by Task 2 (raid event processing)
    pub fn find_open_raid(&self, spt_profile_id: &str) -> rusqlite::Result<Option<Raid>> {
        self.conn
            .query_row(
                "SELECT id, user_id, spt_profile_id, server_id, player_side, faction, map, time_variant, started_at, ended_at, play_time_seconds, exit_status, exit_name, killer_id, killer_aid, xp_before, xp_after, level_before, level_after, victim_count_before
                 FROM raids WHERE spt_profile_id = ?1 AND ended_at IS NULL ORDER BY started_at DESC LIMIT 1",
                params![spt_profile_id],
                row_to_raid,
            )
            .optional()
    }

    #[allow(dead_code)] // Used by raid event handlers
    pub fn insert_raid_snapshot(
        &self,
        raid_id: i64,
        snapshot_type: &str,
        data: &[u8],
    ) -> rusqlite::Result<i64> {
        self.conn.execute(
            "INSERT INTO raid_snapshots (raid_id, snapshot_type, data) VALUES (?1, ?2, ?3)",
            params![raid_id, snapshot_type, data],
        )?;
        Ok(self.conn.last_insert_rowid())
    }

    #[allow(dead_code)] // Used by raid detail handler
    pub fn get_raid_snapshot(
        &self,
        raid_id: i64,
        snapshot_type: &str,
    ) -> rusqlite::Result<Option<Vec<u8>>> {
        self.conn
            .query_row(
                "SELECT data FROM raid_snapshots WHERE raid_id = ?1 AND snapshot_type = ?2",
                params![raid_id, snapshot_type],
                |row| row.get(0),
            )
            .optional()
    }

    #[allow(dead_code)] // Used by raid detail handler
    pub fn get_raid_snapshot_sizes(&self, raid_id: i64) -> rusqlite::Result<Vec<(String, i64)>> {
        let mut stmt = self.conn.prepare(
            "SELECT snapshot_type, length(data) FROM raid_snapshots WHERE raid_id = ?1 ORDER BY snapshot_type",
        )?;
        let rows = stmt.query_map(params![raid_id], |row| Ok((row.get(0)?, row.get(1)?)))?;
        rows.collect()
    }
}

fn row_to_raid(row: &rusqlite::Row<'_>) -> rusqlite::Result<Raid> {
    Ok(Raid {
        id: row.get(0)?,
        user_id: row.get(1)?,
        spt_profile_id: row.get(2)?,
        server_id: row.get(3)?,
        player_side: row.get(4)?,
        faction: row.get(5)?,
        map: row.get(6)?,
        time_variant: row.get(7)?,
        started_at: row.get(8)?,
        ended_at: row.get(9)?,
        play_time_seconds: row.get(10)?,
        exit_status: row.get(11)?,
        exit_name: row.get(12)?,
        killer_id: row.get(13)?,
        killer_aid: row.get(14)?,
        xp_before: row.get(15)?,
        xp_after: row.get(16)?,
        level_before: row.get(17)?,
        level_after: row.get(18)?,
        victim_count_before: row.get(19)?,
    })
}

fn row_to_raid_kill(row: &rusqlite::Row<'_>) -> rusqlite::Result<RaidKill> {
    Ok(RaidKill {
        id: row.get(0)?,
        raid_id: row.get(1)?,
        victim_name: row.get(2)?,
        victim_side: row.get(3)?,
        victim_role: row.get(4)?,
        weapon: row.get(5)?,
        distance: row.get(6)?,
        body_part: row.get(7)?,
        kill_time: row.get(8)?,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::users::Role;

    #[test]
    fn tables_exist() {
        let db = Database::open_in_memory().unwrap();
        let tables: Vec<String> = {
            let mut stmt = db
                .conn()
                .prepare(
                    "SELECT name FROM sqlite_master WHERE type='table' AND name IN ('raids', 'raid_kills', 'raid_snapshots') ORDER BY name",
                )
                .unwrap();
            stmt.query_map([], |row| row.get(0))
                .unwrap()
                .collect::<Result<Vec<String>, _>>()
                .unwrap()
        };
        assert!(tables.contains(&"raids".to_string()));
        assert!(tables.contains(&"raid_kills".to_string()));
        assert!(tables.contains(&"raid_snapshots".to_string()));
    }

    #[test]
    fn insert_and_get_raid() {
        let db = Database::open_in_memory().unwrap();
        let user_id = db
            .insert_user("alice", Some("profile-123"), Some("hash"), Role::Player)
            .unwrap();

        let raid_id = db
            .insert_raid(
                user_id,
                "profile-123",
                Some("server-abc"),
                "Pmc",
                Some("USEC"),
                "Customs",
                Some("day"),
                "2024-01-01T12:00:00Z",
                Some(1000),
                Some(10),
                Some(5),
            )
            .unwrap();
        assert!(raid_id > 0);

        let raids = db.get_raids_for_user(user_id, 10, 0).unwrap();
        assert_eq!(raids.len(), 1);
        let raid = &raids[0];
        assert_eq!(raid.id, raid_id);
        assert_eq!(raid.user_id, user_id);
        assert_eq!(raid.spt_profile_id, "profile-123");
        assert_eq!(raid.server_id.as_deref(), Some("server-abc"));
        assert_eq!(raid.player_side, "Pmc");
        assert_eq!(raid.faction.as_deref(), Some("USEC"));
        assert_eq!(raid.map, "Customs");
        assert_eq!(raid.time_variant.as_deref(), Some("day"));
        assert_eq!(raid.started_at, "2024-01-01T12:00:00Z");
        assert!(raid.ended_at.is_none());
        assert_eq!(raid.xp_before, Some(1000));
        assert_eq!(raid.level_before, Some(10));
        assert_eq!(raid.victim_count_before, Some(5));
    }

    #[test]
    fn close_orphaned_raids_marks_unknown() {
        let db = Database::open_in_memory().unwrap();
        let user_id = db
            .insert_user("bob", Some("profile-456"), Some("hash"), Role::Player)
            .unwrap();

        db.insert_raid(
            user_id,
            "profile-456",
            None,
            "Savage",
            None,
            "Woods",
            None,
            "2024-01-01T10:00:00Z",
            None,
            None,
            None,
        )
        .unwrap();

        let updated = db.close_orphaned_raids("profile-456").unwrap();
        assert_eq!(updated, 1);

        let raids = db.get_raids_for_user(user_id, 10, 0).unwrap();
        assert_eq!(raids.len(), 1);
        assert_eq!(raids[0].exit_status.as_deref(), Some("Unknown"));
        assert!(raids[0].ended_at.is_some());
    }

    #[test]
    fn finish_raid_updates_fields() {
        let db = Database::open_in_memory().unwrap();
        let user_id = db
            .insert_user("charlie", Some("profile-789"), Some("hash"), Role::Player)
            .unwrap();

        let raid_id = db
            .insert_raid(
                user_id,
                "profile-789",
                None,
                "Pmc",
                None,
                "Factory",
                None,
                "2024-01-01T14:00:00Z",
                Some(2000),
                Some(15),
                Some(10),
            )
            .unwrap();

        let updated = db
            .finish_raid(
                raid_id,
                "2024-01-01T14:30:00Z",
                Some(1800),
                "Survived",
                Some("Gate 3"),
                None,
                None,
                Some(2500),
                Some(16),
            )
            .unwrap();
        assert_eq!(updated, 1);

        let raids = db.get_raids_for_user(user_id, 10, 0).unwrap();
        assert_eq!(raids.len(), 1);
        let raid = &raids[0];
        assert_eq!(raid.ended_at.as_deref(), Some("2024-01-01T14:30:00Z"));
        assert_eq!(raid.play_time_seconds, Some(1800));
        assert_eq!(raid.exit_status.as_deref(), Some("Survived"));
        assert_eq!(raid.exit_name.as_deref(), Some("Gate 3"));
        assert_eq!(raid.xp_after, Some(2500));
        assert_eq!(raid.level_after, Some(16));
    }

    #[test]
    fn insert_and_get_kills() {
        let db = Database::open_in_memory().unwrap();
        let user_id = db
            .insert_user("dave", Some("profile-abc"), Some("hash"), Role::Player)
            .unwrap();

        let raid_id = db
            .insert_raid(
                user_id,
                "profile-abc",
                None,
                "Pmc",
                None,
                "Interchange",
                None,
                "2024-01-01T15:00:00Z",
                None,
                None,
                None,
            )
            .unwrap();

        let kills = vec![
            NewRaidKill {
                victim_name: Some("Scav".to_string()),
                victim_side: Some("Savage".to_string()),
                victim_role: Some("assault".to_string()),
                weapon: Some("AK-74".to_string()),
                distance: Some(50.5),
                body_part: Some("Head".to_string()),
                kill_time: Some("2024-01-01T15:05:00Z".to_string()),
            },
            NewRaidKill {
                victim_name: Some("PMC_Bob".to_string()),
                victim_side: Some("Usec".to_string()),
                victim_role: Some("exUsec".to_string()),
                weapon: Some("M4A1".to_string()),
                distance: Some(100.0),
                body_part: Some("Thorax".to_string()),
                kill_time: Some("2024-01-01T15:10:00Z".to_string()),
            },
        ];

        db.insert_raid_kills(raid_id, &kills).unwrap();

        let result = db.get_raid_with_kills(raid_id).unwrap();
        assert!(result.is_some());
        let (raid, raid_kills) = result.unwrap();
        assert_eq!(raid.id, raid_id);
        assert_eq!(raid_kills.len(), 2);
        assert_eq!(raid_kills[0].victim_name.as_deref(), Some("Scav"));
        assert_eq!(raid_kills[0].victim_side.as_deref(), Some("Savage"));
        assert_eq!(raid_kills[0].distance, Some(50.5));
        assert_eq!(raid_kills[1].victim_name.as_deref(), Some("PMC_Bob"));
        assert_eq!(raid_kills[1].weapon.as_deref(), Some("M4A1"));
    }

    #[test]
    fn get_raid_group_returns_squad() {
        let db = Database::open_in_memory().unwrap();
        let user1 = db
            .insert_user("alice", Some("profile-1"), Some("hash"), Role::Player)
            .unwrap();
        let user2 = db
            .insert_user("bob", Some("profile-2"), Some("hash"), Role::Player)
            .unwrap();

        db.insert_raid(
            user1,
            "profile-1",
            Some("squad-123"),
            "Pmc",
            Some("USEC"),
            "Shoreline",
            None,
            "2024-01-01T16:00:00Z",
            None,
            None,
            None,
        )
        .unwrap();

        db.insert_raid(
            user2,
            "profile-2",
            Some("squad-123"),
            "Pmc",
            Some("BEAR"),
            "Shoreline",
            None,
            "2024-01-01T16:00:05Z",
            None,
            None,
            None,
        )
        .unwrap();

        let group = db.get_raid_group("squad-123").unwrap();
        assert_eq!(group.len(), 2);
        assert_eq!(group[0].1, "alice");
        assert_eq!(group[1].1, "bob");
    }

    #[test]
    fn get_active_raids_only_open() {
        let db = Database::open_in_memory().unwrap();
        let user_id = db
            .insert_user("eve", Some("profile-eve"), Some("hash"), Role::Player)
            .unwrap();

        let raid1 = db
            .insert_raid(
                user_id,
                "profile-eve",
                None,
                "Pmc",
                None,
                "Reserve",
                None,
                "2024-01-01T17:00:00Z",
                None,
                None,
                None,
            )
            .unwrap();

        let raid2 = db
            .insert_raid(
                user_id,
                "profile-eve",
                None,
                "Savage",
                None,
                "Labs",
                None,
                "2024-01-01T18:00:00Z",
                None,
                None,
                None,
            )
            .unwrap();

        db.finish_raid(
            raid1,
            "2024-01-01T17:30:00Z",
            Some(1800),
            "Survived",
            None,
            None,
            None,
            None,
            None,
        )
        .unwrap();

        let active = db.get_active_raids().unwrap();
        assert_eq!(active.len(), 1);
        assert_eq!(active[0].0.id, raid2);
        assert_eq!(active[0].1, "eve");
    }

    #[test]
    fn find_open_raid_returns_latest() {
        let db = Database::open_in_memory().unwrap();
        let user_id = db
            .insert_user("frank", Some("profile-frank"), Some("hash"), Role::Player)
            .unwrap();

        db.insert_raid(
            user_id,
            "profile-frank",
            None,
            "Pmc",
            None,
            "Customs",
            None,
            "2024-01-01T10:00:00Z",
            None,
            None,
            None,
        )
        .unwrap();

        let raid2 = db
            .insert_raid(
                user_id,
                "profile-frank",
                None,
                "Savage",
                None,
                "Woods",
                None,
                "2024-01-01T11:00:00Z",
                None,
                None,
                None,
            )
            .unwrap();

        let open = db.find_open_raid("profile-frank").unwrap();
        assert!(open.is_some());
        assert_eq!(open.unwrap().id, raid2);
    }

    #[test]
    fn close_stale_raids_respects_threshold() {
        let db = Database::open_in_memory().unwrap();
        let user_id = db
            .insert_user("grace", Some("profile-grace"), Some("hash"), Role::Player)
            .unwrap();

        // Insert a raid manually with started_at 5 hours ago
        db.conn()
            .execute(
                "INSERT INTO raids (user_id, spt_profile_id, player_side, map, started_at)
                 VALUES (?1, ?2, ?3, ?4, datetime('now', '-5 hours'))",
                params![user_id, "profile-grace", "Pmc", "Lighthouse"],
            )
            .unwrap();

        let updated = db.close_stale_raids(4).unwrap();
        assert_eq!(updated, 1);

        let raids = db.get_raids_for_user(user_id, 10, 0).unwrap();
        assert_eq!(raids.len(), 1);
        assert_eq!(raids[0].exit_status.as_deref(), Some("Unknown"));
        assert!(raids[0].ended_at.is_some());
    }

    #[test]
    fn user_raid_stats_aggregates() {
        let db = Database::open_in_memory().unwrap();
        let user_id = db
            .insert_user("hannah", Some("profile-hannah"), Some("hash"), Role::Player)
            .unwrap();

        // 3 PMC raids: 2 survived, 1 killed
        let pmc1 = db
            .insert_raid(
                user_id,
                "profile-hannah",
                None,
                "Pmc",
                None,
                "Customs",
                None,
                "2024-01-01T10:00:00Z",
                Some(1000),
                Some(10),
                None,
            )
            .unwrap();
        db.finish_raid(
            pmc1,
            "2024-01-01T10:30:00Z",
            Some(1800),
            "Survived",
            None,
            None,
            None,
            Some(1500),
            Some(11),
        )
        .unwrap();
        db.insert_raid_kills(
            pmc1,
            &[NewRaidKill {
                victim_name: Some("Scav1".to_string()),
                victim_side: Some("Savage".to_string()),
                victim_role: None,
                weapon: None,
                distance: None,
                body_part: None,
                kill_time: None,
            }],
        )
        .unwrap();

        let pmc2 = db
            .insert_raid(
                user_id,
                "profile-hannah",
                None,
                "Pmc",
                None,
                "Customs",
                None,
                "2024-01-01T11:00:00Z",
                Some(1500),
                Some(11),
                None,
            )
            .unwrap();
        db.finish_raid(
            pmc2,
            "2024-01-01T11:20:00Z",
            Some(1200),
            "Survived",
            None,
            None,
            None,
            Some(1800),
            Some(12),
        )
        .unwrap();
        db.insert_raid_kills(
            pmc2,
            &[NewRaidKill {
                victim_name: Some("PMC1".to_string()),
                victim_side: Some("Usec".to_string()),
                victim_role: None,
                weapon: None,
                distance: None,
                body_part: None,
                kill_time: None,
            }],
        )
        .unwrap();

        let pmc3 = db
            .insert_raid(
                user_id,
                "profile-hannah",
                None,
                "Pmc",
                None,
                "Woods",
                None,
                "2024-01-01T12:00:00Z",
                Some(1800),
                Some(12),
                None,
            )
            .unwrap();
        db.finish_raid(
            pmc3,
            "2024-01-01T12:15:00Z",
            Some(900),
            "Killed",
            None,
            Some("scav-123"),
            None,
            Some(1900),
            Some(12),
        )
        .unwrap();

        // 1 Scav raid: survived
        let scav1 = db
            .insert_raid(
                user_id,
                "profile-hannah",
                None,
                "Savage",
                None,
                "Interchange",
                None,
                "2024-01-01T13:00:00Z",
                Some(500),
                Some(5),
                None,
            )
            .unwrap();
        db.finish_raid(
            scav1,
            "2024-01-01T13:10:00Z",
            Some(600),
            "Survived",
            None,
            None,
            None,
            Some(600),
            Some(6),
        )
        .unwrap();

        let stats = db.get_user_raid_stats(user_id).unwrap();
        assert_eq!(stats.total_raids, 4);
        assert_eq!(stats.pmc_raids, 3);
        assert_eq!(stats.scav_raids, 1);
        assert!((stats.pmc_survival_rate - 66.666).abs() < 0.01);
        assert!((stats.scav_survival_rate - 100.0).abs() < 0.01);
        assert_eq!(stats.total_kills, 2);
        assert_eq!(stats.pmc_kills, 1);
        assert_eq!(stats.scav_kills, 1);
        assert_eq!(stats.total_deaths, 1);
        assert!((stats.avg_raid_duration - 1125.0).abs() < 0.1);
        assert_eq!(stats.favorite_map.as_deref(), Some("Customs"));
        assert_eq!(stats.pmc_xp_gained, 900); // (1500-1000) + (1800-1500) + (1900-1800)
        assert_eq!(stats.scav_xp_gained, 100); // (600-500)
    }

    #[test]
    fn cascade_delete_user_removes_raids() {
        let db = Database::open_in_memory().unwrap();
        let user_id = db
            .insert_user("ivan", Some("profile-ivan"), Some("hash"), Role::Player)
            .unwrap();

        let raid_id = db
            .insert_raid(
                user_id,
                "profile-ivan",
                None,
                "Pmc",
                None,
                "Factory",
                None,
                "2024-01-01T14:00:00Z",
                None,
                None,
                None,
            )
            .unwrap();

        db.insert_raid_kills(
            raid_id,
            &[NewRaidKill {
                victim_name: Some("Scav".to_string()),
                victim_side: Some("Savage".to_string()),
                victim_role: None,
                weapon: None,
                distance: None,
                body_part: None,
                kill_time: None,
            }],
        )
        .unwrap();

        // Delete user should cascade to raids and raid_kills
        db.conn()
            .execute("DELETE FROM users WHERE id = ?1", params![user_id])
            .unwrap();

        let raids = db.get_raids_for_user(user_id, 10, 0).unwrap();
        assert_eq!(raids.len(), 0);

        let kills = db.get_raid_with_kills(raid_id).unwrap();
        assert!(kills.is_none());
    }

    #[test]
    fn leaderboard_empty_when_no_data() {
        let db = Database::open_in_memory().unwrap();
        let entries = db.get_leaderboard(0).unwrap();
        assert!(entries.is_empty());
    }

    #[test]
    fn leaderboard_respects_min_raids() {
        let db = Database::open_in_memory().unwrap();
        let user1 = db
            .insert_user("alice", Some("p-alice"), Some("hash"), Role::Player)
            .unwrap();
        let user2 = db
            .insert_user("bob", Some("p-bob"), Some("hash"), Role::Player)
            .unwrap();

        // alice: 3 completed raids
        for i in 0..3 {
            let rid = db
                .insert_raid(
                    user1,
                    "p-alice",
                    None,
                    "Pmc",
                    Some("USEC"),
                    "Customs",
                    None,
                    &format!("2024-01-01T{:02}:00:00Z", 10 + i),
                    Some(1000),
                    Some(10),
                    Some(0),
                )
                .unwrap();
            db.finish_raid(
                rid,
                &format!("2024-01-01T{:02}:30:00Z", 10 + i),
                Some(1800),
                "Survived",
                Some("Gate 3"),
                None,
                None,
                Some(1500),
                Some(11),
            )
            .unwrap();
        }

        // bob: 1 completed raid (below threshold of 2)
        let rid = db
            .insert_raid(
                user2,
                "p-bob",
                None,
                "Pmc",
                Some("BEAR"),
                "Woods",
                None,
                "2024-01-01T14:00:00Z",
                Some(500),
                Some(5),
                Some(0),
            )
            .unwrap();
        db.finish_raid(
            rid,
            "2024-01-01T14:30:00Z",
            Some(1800),
            "Killed",
            None,
            Some("scav"),
            None,
            Some(600),
            Some(5),
        )
        .unwrap();

        // min_raids = 2: only alice qualifies
        let entries = db.get_leaderboard(2).unwrap();
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].username, "alice");

        // min_raids = 0: both qualify
        let entries = db.get_leaderboard(0).unwrap();
        assert_eq!(entries.len(), 2);
    }

    #[test]
    fn leaderboard_stats_correct() {
        let db = Database::open_in_memory().unwrap();
        let user_id = db
            .insert_user("carol", Some("p-carol"), Some("hash"), Role::Player)
            .unwrap();

        // Raid 1: Survived, 2 kills (1 headshot), extract "Gate 3"
        let r1 = db
            .insert_raid(
                user_id,
                "p-carol",
                None,
                "Pmc",
                Some("USEC"),
                "Customs",
                None,
                "2024-01-01T10:00:00Z",
                Some(1000),
                Some(10),
                Some(0),
            )
            .unwrap();
        db.finish_raid(
            r1,
            "2024-01-01T10:30:00Z",
            Some(1800),
            "Survived",
            Some("Gate 3"),
            None,
            None,
            Some(1500),
            Some(11),
        )
        .unwrap();
        db.insert_raid_kills(
            r1,
            &[
                NewRaidKill {
                    victim_name: Some("Scav1".to_string()),
                    victim_side: Some("Savage".to_string()),
                    victim_role: Some("assault".to_string()),
                    weapon: Some("AK-74".to_string()),
                    distance: Some(50.0),
                    body_part: Some("Head".to_string()),
                    kill_time: None,
                },
                NewRaidKill {
                    victim_name: Some("PMC1".to_string()),
                    victim_side: Some("Usec".to_string()),
                    victim_role: None,
                    weapon: Some("AK-74".to_string()),
                    distance: Some(150.0),
                    body_part: Some("Thorax".to_string()),
                    kill_time: None,
                },
            ],
        )
        .unwrap();

        // Raid 2: Killed, 1 kill, no extract
        let r2 = db
            .insert_raid(
                user_id,
                "p-carol",
                None,
                "Pmc",
                Some("USEC"),
                "Customs",
                None,
                "2024-01-01T11:00:00Z",
                Some(1500),
                Some(11),
                Some(2),
            )
            .unwrap();
        db.finish_raid(
            r2,
            "2024-01-01T11:15:00Z",
            Some(900),
            "Killed",
            None,
            Some("scav-123"),
            None,
            Some(1600),
            Some(11),
        )
        .unwrap();
        db.insert_raid_kills(
            r2,
            &[NewRaidKill {
                victim_name: Some("Scav2".to_string()),
                victim_side: Some("Savage".to_string()),
                victim_role: Some("assault".to_string()),
                weapon: Some("M4A1".to_string()),
                distance: Some(200.0),
                body_part: Some("Head".to_string()),
                kill_time: None,
            }],
        )
        .unwrap();

        // Raid 3: Survived, 0 kills, extract "Gate 3"
        let r3 = db
            .insert_raid(
                user_id,
                "p-carol",
                None,
                "Pmc",
                Some("USEC"),
                "Woods",
                None,
                "2024-01-01T12:00:00Z",
                Some(1600),
                Some(11),
                Some(3),
            )
            .unwrap();
        db.finish_raid(
            r3,
            "2024-01-01T12:30:00Z",
            Some(1800),
            "Survived",
            Some("Gate 3"),
            None,
            None,
            Some(1800),
            Some(12),
        )
        .unwrap();

        let entries = db.get_leaderboard(0).unwrap();
        assert_eq!(entries.len(), 1);
        let e = &entries[0];

        assert_eq!(e.username, "carol");
        assert_eq!(e.total_raids, 3);
        assert_eq!(e.total_kills, 3);
        assert_eq!(e.total_deaths, 1);
        assert!((e.kd_ratio - 3.0).abs() < 0.01);
        assert!((e.survival_rate - 66.666).abs() < 0.01);
        assert_eq!(e.headshot_count, 2);
        assert!((e.headshot_ratio - 66.666).abs() < 0.01);
        assert!((e.longest_kill - 200.0).abs() < 0.01);
        assert_eq!(e.favorite_weapon.as_deref(), Some("AK-74"));
        assert_eq!(e.favorite_map.as_deref(), Some("Customs"));
        assert_eq!(e.favorite_extract.as_deref(), Some("Gate 3"));
    }

    #[test]
    fn insert_and_get_snapshot() {
        let db = Database::open_in_memory().unwrap();
        let user_id = db
            .insert_user(
                "snap_user",
                Some("profile-snap"),
                Some("hash"),
                Role::Player,
            )
            .unwrap();

        let raid_id = db
            .insert_raid(
                user_id,
                "profile-snap",
                None,
                "Pmc",
                None,
                "Customs",
                None,
                "2024-01-01T12:00:00Z",
                None,
                None,
                None,
            )
            .unwrap();

        let profile_json = br#"{"Info":{"Level":10,"Experience":5000}}"#;
        let compressed = compress_snapshot(profile_json).unwrap();

        let snapshot_id = db
            .insert_raid_snapshot(raid_id, "before", &compressed)
            .unwrap();
        assert!(snapshot_id > 0);

        let retrieved = db.get_raid_snapshot(raid_id, "before").unwrap();
        assert!(retrieved.is_some());
        let decompressed = decompress_snapshot(&retrieved.unwrap()).unwrap();
        assert_eq!(decompressed, profile_json);
    }

    #[test]
    fn snapshot_missing_returns_none() {
        let db = Database::open_in_memory().unwrap();
        let user_id = db
            .insert_user(
                "snap_miss",
                Some("profile-miss"),
                Some("hash"),
                Role::Player,
            )
            .unwrap();

        let raid_id = db
            .insert_raid(
                user_id,
                "profile-miss",
                None,
                "Pmc",
                None,
                "Factory",
                None,
                "2024-01-01T12:00:00Z",
                None,
                None,
                None,
            )
            .unwrap();

        let result = db.get_raid_snapshot(raid_id, "before").unwrap();
        assert!(result.is_none());
    }

    #[test]
    fn snapshot_cascade_deletes_with_raid() {
        let db = Database::open_in_memory().unwrap();
        let user_id = db
            .insert_user(
                "snap_cascade",
                Some("profile-casc"),
                Some("hash"),
                Role::Player,
            )
            .unwrap();

        let raid_id = db
            .insert_raid(
                user_id,
                "profile-casc",
                None,
                "Pmc",
                None,
                "Woods",
                None,
                "2024-01-01T12:00:00Z",
                None,
                None,
                None,
            )
            .unwrap();

        let compressed = compress_snapshot(b"{}").unwrap();
        db.insert_raid_snapshot(raid_id, "before", &compressed)
            .unwrap();
        db.insert_raid_snapshot(raid_id, "after", &compressed)
            .unwrap();

        // Delete the user — cascades to raids, which cascades to snapshots
        db.conn()
            .execute("DELETE FROM users WHERE id = ?1", params![user_id])
            .unwrap();

        let before = db.get_raid_snapshot(raid_id, "before").unwrap();
        assert!(before.is_none());
    }

    #[test]
    fn snapshot_unique_constraint() {
        let db = Database::open_in_memory().unwrap();
        let user_id = db
            .insert_user("snap_dup", Some("profile-dup"), Some("hash"), Role::Player)
            .unwrap();

        let raid_id = db
            .insert_raid(
                user_id,
                "profile-dup",
                None,
                "Pmc",
                None,
                "Reserve",
                None,
                "2024-01-01T12:00:00Z",
                None,
                None,
                None,
            )
            .unwrap();

        let compressed = compress_snapshot(b"{}").unwrap();
        db.insert_raid_snapshot(raid_id, "before", &compressed)
            .unwrap();

        // Second insert with same (raid_id, snapshot_type) should fail
        let result = db.insert_raid_snapshot(raid_id, "before", &compressed);
        assert!(result.is_err());
    }

    #[test]
    fn decompress_corrupt_data_returns_err() {
        let result = decompress_snapshot(b"this is not valid zlib data");
        assert!(result.is_err());
    }

    #[test]
    fn compress_empty_input() {
        let compressed = compress_snapshot(b"").unwrap();
        let decompressed = decompress_snapshot(&compressed).unwrap();
        assert!(decompressed.is_empty());
    }

    #[test]
    fn snapshot_sizes_empty_when_none() {
        let db = Database::open_in_memory().unwrap();
        let user_id = db
            .insert_user(
                "snap_empty",
                Some("profile-empty"),
                Some("hash"),
                Role::Player,
            )
            .unwrap();

        let raid_id = db
            .insert_raid(
                user_id,
                "profile-empty",
                None,
                "Pmc",
                None,
                "Customs",
                None,
                "2024-01-01T12:00:00Z",
                None,
                None,
                None,
            )
            .unwrap();

        let sizes = db.get_raid_snapshot_sizes(raid_id).unwrap();
        assert!(sizes.is_empty());
    }

    #[test]
    fn raid_start_can_store_before_snapshot() {
        let db = Database::open_in_memory().unwrap();
        let user_id = db
            .insert_user(
                "snap_start",
                Some("profile-start"),
                Some("hash"),
                Role::Player,
            )
            .unwrap();

        let raid_id = db
            .insert_raid(
                user_id,
                "profile-start",
                None,
                "Pmc",
                None,
                "Customs",
                None,
                "2024-01-01T12:00:00Z",
                None,
                None,
                None,
            )
            .unwrap();

        // Simulate what handle_raid_start will do: read profile, compress, store
        let profile_json = br#"{"characters":{"pmc":{"Info":{"Level":10}}}}"#;
        let compressed = compress_snapshot(profile_json).unwrap();
        db.insert_raid_snapshot(raid_id, "before", &compressed)
            .unwrap();

        let stored = db.get_raid_snapshot(raid_id, "before").unwrap().unwrap();
        let decompressed = decompress_snapshot(&stored).unwrap();
        assert_eq!(decompressed, profile_json);
    }

    #[test]
    fn raid_end_can_store_after_snapshot() {
        let db = Database::open_in_memory().unwrap();
        let user_id = db
            .insert_user("snap_end", Some("profile-end"), Some("hash"), Role::Player)
            .unwrap();

        let raid_id = db
            .insert_raid(
                user_id,
                "profile-end",
                None,
                "Pmc",
                None,
                "Customs",
                None,
                "2024-01-01T12:00:00Z",
                None,
                None,
                None,
            )
            .unwrap();

        // Simulate what handle_raid_end will do: serialize profile Value, compress, store
        let profile: serde_json::Value =
            serde_json::from_str(r#"{"Info":{"Level":11,"Experience":6000}}"#).unwrap();
        let json_bytes = serde_json::to_vec(&profile).unwrap();
        let compressed = compress_snapshot(&json_bytes).unwrap();
        db.insert_raid_snapshot(raid_id, "after", &compressed)
            .unwrap();

        let stored = db.get_raid_snapshot(raid_id, "after").unwrap().unwrap();
        let decompressed = decompress_snapshot(&stored).unwrap();
        let roundtripped: serde_json::Value = serde_json::from_slice(&decompressed).unwrap();
        assert_eq!(roundtripped["Info"]["Level"], 11);
    }
}
