# Raid Statistics Leaderboard

**Issue:** #49 — Raid statistics and leaderboard
**Date:** 2026-06-20
**Status:** Approved

## Overview

Global leaderboard page displaying ranked player stats from raid data. Combines core combat metrics (kills, K/D, survival rate) with fun stats (headshot ratio, longest kill, favorite weapon/map/extract). Includes wiring up the existing proxy interception hooks so raid data actually flows.

## Scope

### In scope
- Wire proxy hooks for raid start/end event capture
- Global leaderboard page at `/leaderboard` with top-level nav link
- Configurable minimum raid threshold (`leaderboard_min_raids`, default 5)
- All-time stats only

### Out of scope (filed as follow-up issues)
- Per-map leaderboards (#92)
- Head-to-head player comparison (#93)
- Time-scoped leaderboards (monthly, seasonal)

## Architecture

### 1. Proxy Interception Hooks

Add raid event interception to `src/web/proxy.rs`, following the existing `/launcher/profile/register` pattern.

**Endpoints:**
- `POST /client/match/local/start` — clone request body, spawn `handle_raid_start()` on success
- `POST /client/match/local/end` — buffer response body, spawn `handle_raid_end()` on success

**Key details:**
- PHPSESSID cookie contains SPT profile ID, extracted via existing `extract_session_id()`
- Request body is already fully buffered before forwarding
- For raid end: response body must be fully buffered (collect stream to bytes) to pass to `handle_raid_end()`, then re-served to the client
- Both handlers run in `tokio::task::spawn_blocking` since they touch the DB (behind `parking_lot::Mutex`)
- Non-blocking: raid tracking is fire-and-forget (spawned task)
- For raid end only: response switches from streaming to fully-buffered so we can clone the body for tracking. Acceptable tradeoff — raid end responses are small JSON payloads, not large streams

### 2. Leaderboard Data Model

New struct in `src/db/raids.rs`:

```rust
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
```

No schema changes — all derived from existing `raids` + `raid_kills` tables.

### 3. Leaderboard Query

Single CTE-based query in `db.get_leaderboard(min_raids)`:

1. Base CTE: join `raids` → `users`, group by user, compute:
   - `total_raids`, `survival_rate` (survived/total), `total_deaths` (exit_status = 'Killed')
   - `favorite_map` (most common map via window function or subquery)
   - `favorite_extract` (most common exit_name where exit_status = 'Survived')
2. Kills CTE: join `raid_kills` → `raids`, group by user, compute:
   - `total_kills`, `headshot_count` (body_part = 'Head'), `longest_kill` (MAX distance)
   - `favorite_weapon` (most common weapon)
3. Final SELECT: join CTEs, compute `kd_ratio` and `headshot_ratio`, filter by `min_raids` threshold
4. Order by total_kills DESC (default)

### 4. Config

Add to `Config` struct in `src/config.rs`:

```rust
#[serde(default = "default_leaderboard_min_raids")]
pub leaderboard_min_raids: u32,  // default: 5
```

- Default function: `fn default_leaderboard_min_raids() -> u32 { 5 }`
- Env override: `QUMA_LEADERBOARD_MIN_RAIDS`
- TOML key: `leaderboard_min_raids`

### 5. Web Handler

New file `src/web/handlers/leaderboard.rs`:

- `leaderboard_page()` — standard auth → db query → render pattern
- Reads `state.config.leaderboard_min_raids` for threshold
- Passes `Vec<LeaderboardEntry>` + `min_raids` to template
- Register handler module in `src/web/handlers/mod.rs`
- Register route `/leaderboard` in `src/web/mod.rs` authenticated scope

### 6. Template

`templates/leaderboard.html` extending `base.html`:

- Nav active state: `"leaderboard"`
- Header showing threshold ("Players with N+ raids")
- Ranked table with columns: Rank (#), Player, Raids, Kills, Deaths, K/D, Survival %, Headshots, HS%, Longest Kill (m), Fav Weapon, Fav Map, Fav Extract
- Player names link to `/profiles/{username}/raids`
- Pre-sorted by total kills descending
- Uses existing table CSS from style.css

### 7. Navigation

Add "Leaderboard" link to `templates/partials/nav.html` between "Raids" and "ModSync":

```html
<a href="/quma/leaderboard"{% if active == "leaderboard" %} class="active"{% endif %}>
    {% call icons::trophy() %}{% endcall %} Leaderboard
</a>
```

Add `trophy` SVG icon macro to `templates/partials/icons.html` if not already present.

## Testing

### Unit tests (in `db/raids.rs`)
- `leaderboard_respects_min_raids` — users below threshold filtered out
- `leaderboard_stats_correct` — verify K/D, survival rate, headshot ratio, longest kill, favorite weapon/map/extract with known data
- `leaderboard_empty_when_no_data` — no raids returns empty vec

### Manual testing
- Run server via `just serve`, play raid through SPT/Fika, verify data appears on leaderboard
- Verify proxy still functions normally (no regression in SPT client communication)

## File Changes

| File | Change |
|------|--------|
| `src/web/proxy.rs` | Add raid start/end interception |
| `src/db/raids.rs` | Add `LeaderboardEntry` struct + `get_leaderboard()` query |
| `src/config.rs` | Add `leaderboard_min_raids` field + default + env override |
| `src/web/handlers/leaderboard.rs` | New handler file |
| `src/web/handlers/mod.rs` | Register leaderboard module |
| `src/web/mod.rs` | Register `/leaderboard` route |
| `templates/leaderboard.html` | New leaderboard page template |
| `templates/partials/nav.html` | Add Leaderboard nav link |
| `templates/partials/icons.html` | Add trophy icon (if missing) |
