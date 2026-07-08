use std::io::Read;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use actix_web::web::Bytes;
use actix_web::HttpRequest;
use flate2::read::ZlibDecoder;
use serde::{Deserialize, Serialize};

use crate::db::raids::{compress_snapshot, NewRaidKill};
use crate::db::Database;
use crate::web::sse::ServerEvent;

// ── Request/Response Structs ──────────────────────────────────────

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RaidStartRequest {
    pub server_id: Option<String>,
    pub location: String,
    pub time_variant: Option<String>,
    pub player_side: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RaidEndRequest {
    #[allow(dead_code)] // Deserialized from JSON but not read directly
    pub server_id: Option<String>,
    pub results: RaidEndResults,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RaidEndResults {
    pub result: String,
    pub exit_name: Option<String>,
    pub killer_id: Option<String>,
    pub killer_aid: Option<String>,
    pub play_time: Option<i64>,
    pub profile: serde_json::Value,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub struct VictimEntry {
    pub name: Option<String>,
    pub side: Option<String>,
    pub role: Option<String>,
    pub weapon: Option<String>,
    pub distance: Option<f64>,
    pub body_part: Option<String>,
    pub time: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct ProfileSnapshot {
    pub xp: i64,
    pub level: i64,
    pub victim_count: i64,
    pub faction: Option<String>,
}

// ── Helper Functions ──────────────────────────────────────────────

/// Extract PHPSESSID from Cookie header by splitting on `;`, trimming, and finding entry starting with `PHPSESSID=`
pub fn extract_session_id(req: &HttpRequest) -> Option<String> {
    let cookie_header = req.headers().get("cookie")?.to_str().ok()?;
    for entry in cookie_header.split(';') {
        let trimmed = entry.trim();
        if let Some(value) = trimmed.strip_prefix("PHPSESSID=") {
            // SPT session IDs are MongoDB ObjectIDs: exactly 24 hex chars.
            // Reject anything else to prevent path traversal or injection.
            if value.len() == 24 && value.bytes().all(|b| b.is_ascii_hexdigit()) {
                return Some(value.to_string());
            }
            tracing::warn!(value, "rejected invalid PHPSESSID cookie value");
            return None;
        }
    }
    None
}

/// Try zlib decompression, fall back to raw bytes. SPT clients send zlib-compressed request bodies.
fn decompress_body(body: &[u8]) -> Vec<u8> {
    let mut decoder = ZlibDecoder::new(body);
    let mut buf = Vec::new();
    match decoder.read_to_end(&mut buf) {
        Ok(_) => buf,
        Err(_) => body.to_vec(),
    }
}

/// Read on-disk profile JSON and extract XP/level/victim count from the appropriate character (PMC or Scav).
/// Returns `None` if file doesn't exist or can't be parsed.
/// Also returns the raw file bytes for snapshot storage.
pub fn snapshot_profile(
    spt_dir: &Path,
    profile_id: &str,
    is_scav: bool,
) -> Option<(ProfileSnapshot, Vec<u8>)> {
    let path = spt_dir
        .join("SPT/user/profiles")
        .join(format!("{profile_id}.json"));

    let contents = std::fs::read(&path).ok()?;
    let parsed: serde_json::Value = serde_json::from_slice(&contents).ok()?;

    let character_key = if is_scav { "scav" } else { "pmc" };
    let character = parsed.pointer(&format!("/characters/{character_key}"))?;

    let xp = character
        .pointer("/Info/Experience")
        .and_then(|v| v.as_i64())
        .unwrap_or(0);

    let level = character
        .pointer("/Info/Level")
        .and_then(|v| v.as_i64())
        .unwrap_or(1);

    let victims = character
        .pointer("/Stats/Eft/Victims")
        .and_then(|v| v.as_array())
        .map(|arr| arr.len() as i64)
        .unwrap_or(0);

    let faction = if is_scav {
        None
    } else {
        character
            .pointer("/Info/Side")
            .and_then(|v| v.as_str())
            .map(|s| s.to_string())
    };

    Some((
        ProfileSnapshot {
            xp,
            level,
            victim_count: victims,
            faction,
        },
        contents,
    ))
}

// ── Event Handlers ────────────────────────────────────────────────

/// Handle raid start event: parse request, snapshot profile, insert raid row, broadcast event
pub fn handle_raid_start(
    body: Bytes,
    spt_profile_id: String,
    spt_dir: PathBuf,
    db: Arc<parking_lot::Mutex<Database>>,
    events: tokio::sync::broadcast::Sender<ServerEvent>,
    snapshots_enabled: bool,
) {
    let decompressed = decompress_body(&body);
    let start_req: RaidStartRequest = match serde_json::from_slice(&decompressed) {
        Ok(req) => req,
        Err(e) => {
            tracing::warn!(err = %e, profile_id = %spt_profile_id, "failed to parse raid start request");
            return;
        }
    };

    // Determine if this is a scav raid
    let is_scav = start_req.player_side.as_deref() == Some("Savage");

    // Snapshot the profile and compress BEFORE acquiring the lock
    let (snapshot, profile_bytes) = match snapshot_profile(&spt_dir, &spt_profile_id, is_scav) {
        Some(pair) => pair,
        None => {
            tracing::warn!(profile_id = %spt_profile_id, is_scav, "failed to snapshot profile for raid start");
            return;
        }
    };

    let compressed_snapshot = if snapshots_enabled {
        compress_snapshot(&profile_bytes).ok()
    } else {
        None
    };

    let started_at = chrono::Utc::now().to_rfc3339();

    let db_lock = db.lock();

    // Verify user is registered
    let user = match db_lock.get_user_by_spt_profile_id(&spt_profile_id) {
        Ok(Some(u)) => u,
        Ok(None) => {
            tracing::warn!(profile_id = %spt_profile_id, "raid start for unregistered user");
            return;
        }
        Err(e) => {
            tracing::warn!(err = %e, profile_id = %spt_profile_id, "failed to query user by profile ID");
            return;
        }
    };

    // Close any orphaned raids for this profile
    if let Err(e) = db_lock.close_orphaned_raids(&spt_profile_id) {
        tracing::warn!(err = %e, profile_id = %spt_profile_id, "failed to close orphaned raids");
    }

    // Insert raid row
    let raid_id = match db_lock.insert_raid(
        user.id,
        &spt_profile_id,
        start_req.server_id.as_deref(),
        start_req.player_side.as_deref().unwrap_or("Unknown"),
        snapshot.faction.as_deref(),
        &start_req.location,
        start_req.time_variant.as_deref(),
        &started_at,
        Some(snapshot.xp),
        Some(snapshot.level),
        Some(snapshot.victim_count),
    ) {
        Ok(id) => id,
        Err(e) => {
            tracing::warn!(err = %e, profile_id = %spt_profile_id, "failed to insert raid");
            return;
        }
    };

    // Store compressed "before" profile snapshot (best-effort, not transactional with raid insert)
    if let Some(ref compressed) = compressed_snapshot {
        if let Err(e) = db_lock.insert_raid_snapshot(raid_id, "before", compressed) {
            tracing::warn!(err = %e, raid_id, "failed to store before profile snapshot");
        }
    }

    drop(db_lock);

    tracing::info!(
        raid_id,
        profile_id = %spt_profile_id,
        username = %user.username,
        map = %start_req.location,
        player_side = %start_req.player_side.as_deref().unwrap_or("Unknown"),
        "raid started"
    );

    let _ = events.send(ServerEvent::RaidStarted);
}

/// Handle raid end event: parse request, find open raid, extract stats from profile, finish raid, insert kills, broadcast event
pub fn handle_raid_end(
    body: Bytes,
    spt_profile_id: String,
    _spt_dir: PathBuf,
    db: Arc<parking_lot::Mutex<Database>>,
    events: tokio::sync::broadcast::Sender<ServerEvent>,
    snapshots_enabled: bool,
) {
    let decompressed = decompress_body(&body);
    let end_req: RaidEndRequest = match serde_json::from_slice(&decompressed) {
        Ok(req) => req,
        Err(e) => {
            tracing::warn!(err = %e, profile_id = %spt_profile_id, "failed to parse raid end request");
            return;
        }
    };

    let db_lock = db.lock();

    // Find the open raid for this profile
    let open_raid = match db_lock.find_open_raid(&spt_profile_id) {
        Ok(Some(raid)) => raid,
        Ok(None) => {
            tracing::warn!(profile_id = %spt_profile_id, "raid end for profile with no open raid");
            return;
        }
        Err(e) => {
            tracing::warn!(err = %e, profile_id = %spt_profile_id, "failed to query open raid");
            return;
        }
    };

    // Extract stats from the updated profile in the request body
    let profile = &end_req.results.profile;

    // results.profile IS the character data directly (IPmcData), not wrapped in characters.pmc
    let xp_after = profile.pointer("/Info/Experience").and_then(|v| v.as_i64());
    let level_after = profile.pointer("/Info/Level").and_then(|v| v.as_i64());

    // Extract victims array directly from the profile
    let victims: Vec<VictimEntry> = profile
        .pointer("/Stats/Eft/Victims")
        .and_then(|v| serde_json::from_value::<Vec<VictimEntry>>(v.clone()).ok())
        .unwrap_or_default();

    // Diff kills: take victims[victim_count_before..] where victim_count_before comes from the open raid row.
    // Clamp to array length to avoid silent data loss if profile was stale at snapshot time.
    let victim_count_before = open_raid.victim_count_before.unwrap_or(0).max(0) as usize;
    let victim_count_before = victim_count_before.min(victims.len());
    let new_victims: Vec<NewRaidKill> = victims
        .iter()
        .skip(victim_count_before)
        .map(|v| NewRaidKill {
            victim_name: v.name.clone(),
            victim_side: v.side.clone(),
            victim_role: v.role.clone(),
            weapon: v.weapon.clone(),
            distance: v.distance,
            body_part: v.body_part.clone(),
            kill_time: v.time.clone(),
        })
        .collect();

    let ended_at = chrono::Utc::now().to_rfc3339();

    // Finish the raid and insert kills atomically
    if let Err(e) = db_lock.finish_raid_with_kills(
        open_raid.id,
        &ended_at,
        end_req.results.play_time,
        &end_req.results.result,
        end_req.results.exit_name.as_deref(),
        end_req.results.killer_id.as_deref(),
        end_req.results.killer_aid.as_deref(),
        xp_after,
        level_after,
        &new_victims,
    ) {
        tracing::warn!(err = %e, raid_id = open_raid.id, "failed to finish raid");
        return;
    }

    // Store compressed "after" profile snapshot (best-effort, outside the raid transaction)
    if snapshots_enabled {
        if let Ok(json_bytes) = serde_json::to_vec(&end_req.results.profile) {
            if let Ok(compressed) = compress_snapshot(&json_bytes) {
                if let Err(e) = db_lock.insert_raid_snapshot(open_raid.id, "after", &compressed) {
                    tracing::warn!(err = %e, raid_id = open_raid.id, "failed to store after profile snapshot");
                }
            }
        }
    }

    drop(db_lock);

    tracing::info!(
        raid_id = open_raid.id,
        profile_id = %spt_profile_id,
        exit_status = %end_req.results.result,
        kills = new_victims.len(),
        "raid ended"
    );

    let _ = events.send(ServerEvent::RaidEnded);
}

#[cfg(test)]
mod tests {
    use super::*;
    use actix_web::test::TestRequest;

    fn req_with_session(value: &str) -> HttpRequest {
        TestRequest::default()
            .insert_header(("cookie", format!("PHPSESSID={value}")))
            .to_http_request()
    }

    #[test]
    fn valid_session_id_accepted() {
        let req = req_with_session("aabbccdd11223344aabbccdd");
        assert_eq!(
            extract_session_id(&req),
            Some("aabbccdd11223344aabbccdd".to_string())
        );
    }

    #[test]
    fn path_traversal_rejected() {
        let req = req_with_session("../../etc/passwd");
        assert_eq!(extract_session_id(&req), None);
    }

    #[test]
    fn too_short_rejected() {
        let req = req_with_session("aabbcc");
        assert_eq!(extract_session_id(&req), None);
    }

    #[test]
    fn non_hex_rejected() {
        let req = req_with_session("zzzzzzzzzzzzzzzzzzzzzzzz");
        assert_eq!(extract_session_id(&req), None);
    }

    #[test]
    fn raid_start_parses_null_server_id() {
        let json = r#"{"serverId":null,"location":"factory4_day","timeVariant":"PAST","playerSide":"Pmc"}"#;
        let req: RaidStartRequest = serde_json::from_str(json).unwrap();
        assert!(req.server_id.is_none());
        assert_eq!(req.location, "factory4_day");
    }

    #[test]
    fn raid_start_parses_present_server_id() {
        let json = r#"{"serverId":"abc123","location":"Customs"}"#;
        let req: RaidStartRequest = serde_json::from_str(json).unwrap();
        assert_eq!(req.server_id.as_deref(), Some("abc123"));
    }
}
