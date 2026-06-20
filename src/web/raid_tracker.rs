use std::path::{Path, PathBuf};
use std::sync::Arc;

use actix_web::web::Bytes;
use actix_web::HttpRequest;
use serde::{Deserialize, Serialize};

use crate::db::raids::NewRaidKill;
use crate::db::Database;
use crate::web::sse::ServerEvent;

// ── Request/Response Structs ──────────────────────────────────────

#[allow(dead_code)] // Used by Task 5 (proxy handler)
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RaidStartRequest {
    pub server_id: String,
    pub location: String,
    pub time_variant: Option<String>,
    pub player_side: Option<String>,
}

#[allow(dead_code)] // Used by Task 6 (proxy handler)
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RaidEndRequest {
    pub server_id: Option<String>,
    pub results: RaidEndResults,
}

#[allow(dead_code)] // Used by Task 6 (proxy handler)
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

#[allow(dead_code)] // Used by Task 6 (proxy handler)
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

#[allow(dead_code)] // Used by Task 5-6 (raid event processing)
#[derive(Debug, Serialize)]
pub struct ProfileSnapshot {
    pub xp: i64,
    pub level: i64,
    pub victim_count: i64,
    pub faction: Option<String>,
}

// ── Helper Functions ──────────────────────────────────────────────

/// Extract PHPSESSID from Cookie header by splitting on `;`, trimming, and finding entry starting with `PHPSESSID=`
#[allow(dead_code)] // Used by Task 5-6 (proxy handlers)
pub fn extract_session_id(req: &HttpRequest) -> Option<String> {
    let cookie_header = req.headers().get("cookie")?.to_str().ok()?;
    for entry in cookie_header.split(';') {
        let trimmed = entry.trim();
        if let Some(value) = trimmed.strip_prefix("PHPSESSID=") {
            return Some(value.to_string());
        }
    }
    None
}

/// Read on-disk profile JSON and extract XP/level/victim count from the appropriate character (PMC or Scav).
/// Returns `None` if file doesn't exist or can't be parsed.
#[allow(dead_code)] // Used by Task 5 (raid start handler)
pub fn snapshot_profile(
    spt_dir: &Path,
    profile_id: &str,
    is_scav: bool,
) -> Option<ProfileSnapshot> {
    let path = spt_dir
        .join("SPT/user/profiles")
        .join(format!("{profile_id}.json"));

    let contents = std::fs::read_to_string(&path).ok()?;
    let parsed: serde_json::Value = serde_json::from_str(&contents).ok()?;

    let character_key = if is_scav { "savage" } else { "pmc" };
    let character = parsed
        .pointer(&format!("/characters/{character_key}"))
        .or_else(|| {
            // Fallback: try "Savage" with capital S
            if is_scav {
                parsed.pointer("/characters/Savage")
            } else {
                None
            }
        })?;

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

    Some(ProfileSnapshot {
        xp,
        level,
        victim_count: victims,
        faction,
    })
}

// ── Event Handlers ────────────────────────────────────────────────

/// Handle raid start event: parse request, snapshot profile, insert raid row, broadcast event
#[allow(dead_code)] // Used by Task 5 (proxy handler)
pub fn handle_raid_start(
    body: Bytes,
    spt_profile_id: String,
    spt_dir: PathBuf,
    db: Arc<parking_lot::Mutex<Database>>,
    events: tokio::sync::broadcast::Sender<ServerEvent>,
) {
    let start_req: RaidStartRequest = match serde_json::from_slice(&body) {
        Ok(req) => req,
        Err(e) => {
            tracing::warn!(error = %e, profile_id = %spt_profile_id, "failed to parse raid start request");
            return;
        }
    };

    let db_lock = db.lock();

    // Verify user is registered
    let user = match db_lock.get_user_by_spt_profile_id(&spt_profile_id) {
        Ok(Some(u)) => u,
        Ok(None) => {
            tracing::warn!(profile_id = %spt_profile_id, "raid start for unregistered user");
            return;
        }
        Err(e) => {
            tracing::warn!(error = %e, profile_id = %spt_profile_id, "failed to query user by profile ID");
            return;
        }
    };

    // Close any orphaned raids for this profile
    if let Err(e) = db_lock.close_orphaned_raids(&spt_profile_id) {
        tracing::warn!(error = %e, profile_id = %spt_profile_id, "failed to close orphaned raids");
    }

    // Determine if this is a scav raid
    let is_scav = start_req.player_side.as_deref() == Some("Savage");

    // Snapshot the profile to get baseline stats
    let snapshot = match snapshot_profile(&spt_dir, &spt_profile_id, is_scav) {
        Some(s) => s,
        None => {
            tracing::warn!(profile_id = %spt_profile_id, is_scav, "failed to snapshot profile for raid start");
            return;
        }
    };

    let started_at = chrono::Utc::now().to_rfc3339();

    // Insert raid row
    let raid_id = match db_lock.insert_raid(
        user.id,
        &spt_profile_id,
        Some(&start_req.server_id),
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
            tracing::warn!(error = %e, profile_id = %spt_profile_id, "failed to insert raid");
            return;
        }
    };

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
#[allow(dead_code)] // Used by Task 6 (proxy handler)
pub fn handle_raid_end(
    body: Bytes,
    spt_profile_id: String,
    _spt_dir: PathBuf,
    db: Arc<parking_lot::Mutex<Database>>,
    events: tokio::sync::broadcast::Sender<ServerEvent>,
) {
    let end_req: RaidEndRequest = match serde_json::from_slice(&body) {
        Ok(req) => req,
        Err(e) => {
            tracing::warn!(error = %e, profile_id = %spt_profile_id, "failed to parse raid end request");
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
            tracing::warn!(error = %e, profile_id = %spt_profile_id, "failed to query open raid");
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

    // Diff kills: take victims[victim_count_before..] where victim_count_before comes from the open raid row
    let victim_count_before = open_raid.victim_count_before.unwrap_or(0) as usize;
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

    // Finish the raid
    if let Err(e) = db_lock.finish_raid(
        open_raid.id,
        &ended_at,
        end_req.results.play_time,
        &end_req.results.result,
        end_req.results.exit_name.as_deref(),
        end_req.results.killer_id.as_deref(),
        end_req.results.killer_aid.as_deref(),
        xp_after,
        level_after,
    ) {
        tracing::warn!(error = %e, raid_id = open_raid.id, "failed to finish raid");
        return;
    }

    // Insert kills
    if !new_victims.is_empty() {
        if let Err(e) = db_lock.insert_raid_kills(open_raid.id, &new_victims) {
            tracing::warn!(error = %e, raid_id = open_raid.id, "failed to insert raid kills");
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
