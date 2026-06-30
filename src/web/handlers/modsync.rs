use std::collections::BTreeMap;

use actix_session::Session;
use actix_web::web::{self, Data, Form, Json, Query};
use actix_web::{HttpRequest, HttpResponse};
use askama::Template;

use crate::config::{validate_group_slug, Config, ModSyncGroup, NARCONET_FORGE_MOD_ID};
use crate::db::rbac::Permission;
use crate::web::auth::{require_auth, require_permission, SessionUser};
use crate::web::error::WebError;
use crate::web::flash::{set_flash, take_flash, FlashMessage, FlashType};
use crate::web::nav::NavContext;
use crate::web::state::AppState;

#[allow(unused_imports)]
mod filters {
    pub use crate::web::template_filters::*;
}

#[derive(serde::Deserialize)]
pub struct ModsyncQuery {
    #[serde(default = "default_tab")]
    tab: String,
}

fn default_tab() -> String {
    "settings".to_string()
}

#[derive(Template)]
#[template(path = "modsync.html")]
struct ModSyncTemplate {
    user: SessionUser,
    flash: Option<FlashMessage>,
    csrf_token: String,
    nav: NavContext,
    modsync_managed: bool,
    active_tab: String,
    tab_content: String,
}

#[derive(Template)]
#[template(path = "modsync/partials/settings.html")]
struct SettingsPartialTemplate {
    csrf_token: String,
    enforced: bool,
    silent: bool,
    restart_required: bool,
    extra_sync_paths: String,
    exclusions: String,
}

pub async fn modsync_page(
    state: Data<AppState>,
    req: HttpRequest,
    session: Session,
    query: Query<ModsyncQuery>,
) -> actix_web::Result<HttpResponse> {
    let user = require_auth(&req)?;
    require_permission(&user, Permission::ModsyncManage)?;
    let flash = take_flash(&session);
    let csrf_token = crate::web::csrf::get_or_create_token(&session);

    let ms_config = state.config().modsync.clone();

    let nav = NavContext::from_state(&state);
    let modsync_managed = nav.modsync_installed && ms_config.is_some();

    // Determine active tab (validate and constrain based on state)
    let mut active_tab = query.tab.as_str();
    let valid_tabs = ["settings", "groups", "mods", "preview"];
    if !valid_tabs.contains(&active_tab) {
        active_tab = "settings";
    }
    // If not managed, force to settings
    if !modsync_managed && active_tab != "settings" {
        active_tab = "settings";
    }

    // Render the appropriate tab partial
    let tab_content = match active_tab {
        "settings" => {
            let (enforced, silent, restart_required, extra_sync_paths, exclusions) =
                if let Some(ref ms) = ms_config {
                    (
                        ms.enforced,
                        ms.silent,
                        ms.restart_required,
                        ms.extra_sync_paths.join("\n"),
                        ms.exclusions.join("\n"),
                    )
                } else {
                    (true, false, true, String::new(), String::new())
                };
            let partial = SettingsPartialTemplate {
                csrf_token: csrf_token.clone(),
                enforced,
                silent,
                restart_required,
                extra_sync_paths,
                exclusions,
            };
            partial.render().map_err(WebError::from)?
        }
        "groups" => render_groups_tab(&state, &csrf_token).await?,
        "mods" => render_mods_tab(&state, &csrf_token).await?,
        "preview" => render_preview_tab(&state).await?,
        _ => "<p>Unknown tab</p>".to_string(),
    };

    let tmpl = ModSyncTemplate {
        user,
        flash,
        csrf_token,
        nav,
        modsync_managed,
        active_tab: active_tab.to_string(),
        tab_content,
    };
    Ok(HttpResponse::Ok()
        .content_type("text/html; charset=utf-8")
        .body(tmpl.render().map_err(WebError::from)?))
}

pub async fn settings_partial(
    state: Data<AppState>,
    req: HttpRequest,
    session: Session,
) -> actix_web::Result<HttpResponse> {
    let user = require_auth(&req)?;
    require_permission(&user, Permission::ModsyncManage)?;
    let csrf_token = crate::web::csrf::get_or_create_token(&session);

    let ms_config = state.config().modsync.clone();

    let (enforced, silent, restart_required, extra_sync_paths, exclusions) =
        if let Some(ref ms) = ms_config {
            (
                ms.enforced,
                ms.silent,
                ms.restart_required,
                ms.extra_sync_paths.join("\n"),
                ms.exclusions.join("\n"),
            )
        } else {
            (true, false, true, String::new(), String::new())
        };

    let tmpl = SettingsPartialTemplate {
        csrf_token,
        enforced,
        silent,
        restart_required,
        extra_sync_paths,
        exclusions,
    };
    Ok(HttpResponse::Ok()
        .content_type("text/html; charset=utf-8")
        .body(tmpl.render().map_err(WebError::from)?))
}

#[derive(serde::Deserialize)]
pub struct ModSyncSettingsForm {
    csrf_token: String,
    enforced: Option<String>,
    silent: Option<String>,
    restart_required: Option<String>,
    extra_sync_paths: String,
    exclusions: String,
}

pub async fn save_settings(
    state: Data<AppState>,
    req: HttpRequest,
    session: Session,
    form: Form<ModSyncSettingsForm>,
) -> actix_web::Result<HttpResponse> {
    let user = require_auth(&req)?;
    require_permission(&user, Permission::ModsyncManage)?;
    if !crate::web::csrf::validate_token(&session, &form.csrf_token) {
        return Err(WebError::Forbidden.into());
    }

    let extra_paths: Vec<String> = form
        .extra_sync_paths
        .lines()
        .map(|l| l.trim().to_string())
        .filter(|l| !l.is_empty())
        .collect();

    let exclusion_list: Vec<String> = form
        .exclusions
        .lines()
        .map(|l| l.trim().to_string())
        .filter(|l| !l.is_empty())
        .collect();

    let _guard = state.config_lock.lock();
    let mut new_config = Config::load(&state.config_path).map_err(WebError::from)?;
    // Preserve existing overrides and groups
    let existing_overrides = new_config
        .modsync
        .as_ref()
        .map(|ms| ms.overrides.clone())
        .unwrap_or_default();
    let existing_groups = new_config
        .modsync
        .as_ref()
        .map(|ms| ms.groups.clone())
        .unwrap_or_default();
    // Preserve existing enabled flag (Task 2 will add UI for this)
    let existing_enabled = new_config
        .modsync
        .as_ref()
        .map(|ms| ms.enabled)
        .unwrap_or(true);

    let ms_config = crate::config::ModSyncConfig {
        enabled: existing_enabled,
        enforced: form.enforced.is_some(),
        silent: form.silent.is_some(),
        restart_required: form.restart_required.is_some(),
        extra_sync_paths: extra_paths,
        exclusions: exclusion_list,
        overrides: existing_overrides,
        groups: existing_groups,
    };

    // Update config and save
    new_config.modsync = Some(ms_config);
    new_config
        .save(&state.config_path)
        .map_err(WebError::from)?;
    if let Err(e) = state.update_config_from_disk() {
        tracing::warn!(err = %e, "failed to refresh in-memory config after save");
    }
    drop(_guard);

    // Regenerate NarcoNet config.yaml
    if state.is_modsync_installed() {
        let db = state.db.clone();
        let spt_dir = state.spt_dir.clone();
        let config = new_config.clone();
        let _ = web::block(move || {
            let db = db.lock();
            crate::modsync::regenerate_if_enabled(&spt_dir, &config, &db)
        })
        .await;
    }

    set_flash(&session, "NarcoNet settings saved", FlashType::Success);
    Ok(HttpResponse::SeeOther()
        .insert_header(("Location", "/quma/modsync?tab=settings"))
        .finish())
}

// ─── Groups Tab ─────────────────────────────────────────────────────────────

/// Info about a mod available for group assignment.
#[derive(Clone)]
struct AvailableMod {
    forge_id: i64,
    name: String,
}

/// A resolved member badge in a group card.
struct GroupMember {
    forge_id: i64,
    name: String,
    installed: bool,
    has_client_files: bool,
}

/// Context for rendering a single group card.
#[derive(Template)]
#[template(path = "modsync/partials/group_card.html")]
struct GroupCardTemplate {
    index: usize,
    display_name: String,
    slug: String,
    enabled_val: String,
    enforced_val: String,
    silent_val: String,
    restart_required_val: String,
    exclude_headless: bool,
    members: Vec<GroupMember>,
    available_mods: Vec<AvailableMod>,
}

#[derive(Template)]
#[template(path = "modsync/partials/groups.html")]
struct GroupsPartialTemplate {
    csrf_token: String,
    groups: Vec<GroupCardTemplate>,
    next_index: usize,
}

/// Parse a JSON field as an `Option<bool>` three-state value.
/// `"true"` → `Some(true)`, `"false"` → `Some(false)`, anything else → `None` (inherit).
fn parse_opt_bool(val: &serde_json::Value, key: &str) -> Option<bool> {
    val.get(key)
        .and_then(|v| v.as_str())
        .map(|s| match s {
            "true" => Some(true),
            "false" => Some(false),
            _ => None, // empty string = default/inherit
        })
        .unwrap_or(None)
}

/// Convert an Option<bool> to three-state string for template select.
fn opt_bool_to_val(v: Option<bool>) -> String {
    match v {
        Some(true) => "true".to_string(),
        Some(false) => "false".to_string(),
        None => String::new(),
    }
}

/// Identify which installed mods have at least one BepInEx/ client file.
/// Takes a `Database` reference directly so callers can pass a locked DB from within `web::block`.
fn mods_with_client_files(
    db: &crate::db::Database,
) -> Result<Vec<(i64, String, bool)>, anyhow::Error> {
    let mods = db.list_mods()?;
    let mut result = Vec::new();
    for m in &mods {
        if m.forge_mod_id == NARCONET_FORGE_MOD_ID {
            continue;
        }
        let files = db.get_files_for_mod(m.id)?;
        let has_client = files.iter().any(|f| f.file_path.starts_with("BepInEx/"));
        result.push((m.forge_mod_id, m.name.clone(), has_client));
    }
    Ok(result)
}

/// Fetch mods_with_client_files via web::block (async-safe).
async fn fetch_mods_with_client_files(
    state: &AppState,
) -> Result<Vec<(i64, String, bool)>, WebError> {
    let db = state.db.clone();
    web::block(move || {
        let db = db.lock();
        mods_with_client_files(&db)
    })
    .await
    .map_err(WebError::from)?
    .map_err(WebError::from)
}

/// Shared logic: build the groups tab HTML from config + DB state.
async fn render_groups_tab(state: &AppState, csrf_token: &str) -> Result<String, WebError> {
    let ms_config = state.config().modsync.clone().unwrap_or_default();

    let all_mods = fetch_mods_with_client_files(state).await?;

    // Build set of assigned mod IDs across all groups
    let mut assigned: std::collections::HashSet<i64> = std::collections::HashSet::new();
    for group in ms_config.groups.values() {
        for &id in &group.members {
            assigned.insert(id);
        }
    }

    // Mod lookup by forge_id for resolving member names
    let mod_lookup: std::collections::HashMap<i64, (&str, bool)> = all_mods
        .iter()
        .map(|(id, name, has_client)| (*id, (name.as_str(), *has_client)))
        .collect();

    let mut group_cards: Vec<GroupCardTemplate> = Vec::new();
    for (idx, (slug, group)) in ms_config.groups.iter().enumerate() {
        let members: Vec<GroupMember> = group
            .members
            .iter()
            .map(|&forge_id| {
                if let Some(&(name, has_client)) = mod_lookup.get(&forge_id) {
                    GroupMember {
                        forge_id,
                        name: name.to_string(),
                        installed: true,
                        has_client_files: has_client,
                    }
                } else {
                    GroupMember {
                        forge_id,
                        name: format!("Mod #{forge_id}"),
                        installed: false,
                        has_client_files: false,
                    }
                }
            })
            .collect();

        // For this card, available = global available + mods assigned to THIS group
        // (since this group already "owns" them)
        let card_available: Vec<AvailableMod> = all_mods
            .iter()
            .filter(|(id, _, has_client)| {
                *has_client && (!assigned.contains(id) || group.members.contains(id))
            })
            .map(|(id, name, _)| AvailableMod {
                forge_id: *id,
                name: name.clone(),
            })
            .collect();

        group_cards.push(GroupCardTemplate {
            index: idx,
            display_name: group.display_name.clone(),
            slug: slug.clone(),
            enabled_val: opt_bool_to_val(group.enabled),
            enforced_val: opt_bool_to_val(group.enforced),
            silent_val: opt_bool_to_val(group.silent),
            restart_required_val: opt_bool_to_val(group.restart_required),
            exclude_headless: group.exclude_headless,
            members,
            available_mods: card_available,
        });
    }

    let next_index = group_cards.len();
    let tmpl = GroupsPartialTemplate {
        csrf_token: csrf_token.to_string(),
        groups: group_cards,
        next_index,
    };
    tmpl.render().map_err(WebError::from)
}

pub async fn groups_partial(
    state: Data<AppState>,
    req: HttpRequest,
    session: Session,
) -> actix_web::Result<HttpResponse> {
    let user = require_auth(&req)?;
    require_permission(&user, Permission::ModsyncManage)?;
    let csrf_token = crate::web::csrf::get_or_create_token(&session);

    let html = render_groups_tab(&state, &csrf_token).await?;
    Ok(HttpResponse::Ok()
        .content_type("text/html; charset=utf-8")
        .body(html))
}

#[derive(serde::Deserialize)]
pub struct NewGroupQuery {
    #[serde(default)]
    index: usize,
}

pub async fn new_group_card(
    state: Data<AppState>,
    req: HttpRequest,
    session: Session,
    query: Query<NewGroupQuery>,
) -> actix_web::Result<HttpResponse> {
    let user = require_auth(&req)?;
    require_permission(&user, Permission::ModsyncManage)?;
    let _csrf_token = crate::web::csrf::get_or_create_token(&session);

    let all_mods = fetch_mods_with_client_files(&state).await?;

    // Load current config to know which mods are already assigned
    let ms_config = state.config().modsync.clone().unwrap_or_default();

    let mut assigned: std::collections::HashSet<i64> = std::collections::HashSet::new();
    for group in ms_config.groups.values() {
        for &id in &group.members {
            assigned.insert(id);
        }
    }

    let available: Vec<AvailableMod> = all_mods
        .iter()
        .filter(|(id, _, has_client)| *has_client && !assigned.contains(id))
        .map(|(id, name, _)| AvailableMod {
            forge_id: *id,
            name: name.clone(),
        })
        .collect();

    let tmpl = GroupCardTemplate {
        index: query.index,
        display_name: String::new(),
        slug: String::new(),
        enabled_val: String::new(),
        enforced_val: String::new(),
        silent_val: String::new(),
        restart_required_val: String::new(),
        exclude_headless: false,
        members: Vec::new(),
        available_mods: available,
    };

    Ok(HttpResponse::Ok()
        .content_type("text/html; charset=utf-8")
        .body(tmpl.render().map_err(WebError::from)?))
}

pub async fn save_groups(
    state: Data<AppState>,
    req: HttpRequest,
    session: Session,
    body: Json<serde_json::Value>,
) -> actix_web::Result<HttpResponse> {
    let user = require_auth(&req)?;
    require_permission(&user, Permission::ModsyncManage)?;

    let csrf = body
        .get("csrf_token")
        .and_then(|v| v.as_str())
        .unwrap_or("");
    if !crate::web::csrf::validate_token(&session, csrf) {
        return Err(WebError::Forbidden.into());
    }

    let groups_val = body
        .get("groups")
        .and_then(|v| v.as_array())
        .cloned()
        .unwrap_or_default();

    // Build the set of mods with client files for validation
    let all_mods = fetch_mods_with_client_files(&state).await?;
    let client_mod_ids: std::collections::HashSet<i64> = all_mods
        .iter()
        .filter(|(_, _, has_client)| *has_client)
        .map(|(id, _, _)| *id)
        .collect();

    let mut new_groups: BTreeMap<String, ModSyncGroup> = BTreeMap::new();

    for group_val in &groups_val {
        let display_name = group_val
            .get("display_name")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .trim()
            .to_string();

        if display_name.is_empty() {
            continue; // Skip groups with no name
        }

        let slug = group_val
            .get("slug")
            .and_then(|v| v.as_str())
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty())
            .unwrap_or_else(|| crate::config::slugify(&display_name));

        if validate_group_slug(&slug).is_err() {
            continue; // Skip groups with invalid slugs
        }

        // Parse members, keeping only valid mod IDs with client files
        let members: Vec<i64> = group_val
            .get("members")
            .and_then(|v| v.as_array())
            .map(|arr| {
                arr.iter()
                    .filter_map(|v| v.as_i64())
                    .filter(|id| client_mod_ids.contains(id))
                    .collect()
            })
            .unwrap_or_default();

        let group = ModSyncGroup {
            display_name,
            members,
            enabled: parse_opt_bool(group_val, "enabled"),
            enforced: parse_opt_bool(group_val, "enforced"),
            silent: parse_opt_bool(group_val, "silent"),
            restart_required: parse_opt_bool(group_val, "restart_required"),
            exclude_headless: group_val
                .get("exclude_headless")
                .and_then(|v| v.as_bool())
                .unwrap_or(false),
        };

        // Deduplicate slug: if duplicate, append a suffix
        let mut final_slug = slug;
        if new_groups.contains_key(&final_slug) {
            let mut counter = 2;
            loop {
                let candidate = format!("{final_slug}-{counter}");
                if !new_groups.contains_key(&candidate) {
                    final_slug = candidate;
                    break;
                }
                counter += 1;
            }
        }

        new_groups.insert(final_slug, group);
    }

    // Dedup: if a mod appears in multiple groups, keep only the first occurrence
    {
        let mut seen: std::collections::HashSet<i64> = std::collections::HashSet::new();
        for group in new_groups.values_mut() {
            group.members.retain(|id| seen.insert(*id));
        }
    }

    // Load-then-mutate: only replace the groups field
    let _guard = state.config_lock.lock();
    let mut config = Config::load(&state.config_path).map_err(WebError::from)?;
    if let Some(ref mut ms) = config.modsync {
        ms.groups = new_groups;
    } else {
        config.modsync = Some(crate::config::ModSyncConfig {
            groups: new_groups,
            ..crate::config::ModSyncConfig::default()
        });
    }
    config.save(&state.config_path).map_err(WebError::from)?;
    if let Err(e) = state.update_config_from_disk() {
        tracing::warn!(err = %e, "failed to refresh in-memory config after save");
    }
    drop(_guard);

    // Regenerate NarcoNet config.yaml
    if state.is_modsync_installed() {
        let db = state.db.clone();
        let spt_dir = state.spt_dir.clone();
        let saved_config = config.clone();
        let _ = web::block(move || {
            let db = db.lock();
            crate::modsync::regenerate_if_enabled(&spt_dir, &saved_config, &db)
        })
        .await;
    }

    set_flash(&session, "NarcoNet groups saved", FlashType::Success);
    Ok(HttpResponse::Ok().json(serde_json::json!({
        "ok": true,
        "redirect": "/quma/modsync?tab=groups"
    })))
}

// ─── Mods Tab ───────────────────────────────────────────────────────────────

/// Context for a single mod row in the overrides table.
struct ModOverrideRow {
    db_id: i64,
    forge_mod_id: i64,
    name: String,
    group: Option<String>,
    override_enabled: String,
    override_enforced: String,
    override_silent: String,
    override_restart_required: String,
    effective_enabled: bool,
    effective_enforced: bool,
    effective_silent: bool,
    effective_restart_required: bool,
    override_conflict_enabled: bool,
    override_conflict_enforced: bool,
    override_conflict_silent: bool,
    override_conflict_restart_required: bool,
    headless_warning: bool,
    headless_disabled: bool,
}

#[derive(Template)]
#[template(path = "modsync/partials/mods.html")]
struct ModsPartialTemplate {
    csrf_token: String,
    mods: Vec<ModOverrideRow>,
}

/// Build a reverse group membership map: forge_mod_id -> group slug
fn build_group_membership_map(
    config: &crate::config::ModSyncConfig,
) -> std::collections::HashMap<i64, String> {
    let mut map = std::collections::HashMap::new();
    for (slug, group) in &config.groups {
        for &member_id in &group.members {
            map.insert(member_id, slug.clone());
        }
    }
    map
}

/// Compute effective values from cascade: global → group → per-mod override.
/// Returns (enabled, enforced, silent, restart_required).
fn compute_effective_values(
    global: &crate::config::ModSyncConfig,
    group: Option<&crate::config::ModSyncGroup>,
    override_val: Option<&crate::config::ModSyncOverride>,
) -> (bool, bool, bool, bool) {
    let enabled = override_val
        .and_then(|o| o.enabled)
        .or_else(|| group.and_then(|g| g.enabled))
        .unwrap_or(true); // Default enabled if not specified anywhere

    let enforced = override_val
        .and_then(|o| o.enforced)
        .or_else(|| group.and_then(|g| g.enforced))
        .unwrap_or(global.enforced);

    let silent = override_val
        .and_then(|o| o.silent)
        .or_else(|| group.and_then(|g| g.silent))
        .unwrap_or(global.silent);

    let restart_required = override_val
        .and_then(|o| o.restart_required)
        .or_else(|| group.and_then(|g| g.restart_required))
        .unwrap_or(global.restart_required);

    (enabled, enforced, silent, restart_required)
}

/// Shared logic: build the mods tab HTML from config + DB state.
async fn render_mods_tab(state: &AppState, csrf_token: &str) -> Result<String, WebError> {
    let ms_config = state.config().modsync.clone().unwrap_or_default();

    // Fetch all DB data in a single web::block call: mod list + forge_id → db_id mapping
    let db = state.db.clone();
    let all_mods_and_ids = web::block(move || {
        let db = db.lock();
        let all_mods = mods_with_client_files(&db)?;
        let mut forge_to_db_id: std::collections::HashMap<i64, i64> =
            std::collections::HashMap::new();
        for (forge_id, _, has_client) in &all_mods {
            if !has_client {
                continue;
            }
            if let Some(m) = db.get_mod_by_forge_id(*forge_id)? {
                forge_to_db_id.insert(*forge_id, m.id);
            }
        }
        Ok::<_, anyhow::Error>((all_mods, forge_to_db_id))
    })
    .await
    .map_err(WebError::from)?
    .map_err(WebError::from)?;

    let (all_mods, forge_to_db_id) = all_mods_and_ids;
    let group_map = build_group_membership_map(&ms_config);

    // Build rows (no DB access needed here — all data pre-fetched)
    let mut rows = Vec::new();
    for (forge_id, name, has_client) in &all_mods {
        if !has_client {
            continue;
        }

        let db_id = match forge_to_db_id.get(forge_id) {
            Some(&id) => id,
            None => continue,
        };

        let group_slug = group_map.get(forge_id);
        let group_obj = group_slug.and_then(|slug| ms_config.groups.get(slug));

        let override_val = ms_config.overrides.get(&forge_id.to_string());

        let (eff_enabled, eff_enforced, eff_silent, eff_restart_required) =
            compute_effective_values(&ms_config, group_obj, override_val);

        // Determine group-level effective (what would be inherited if no per-mod override)
        let (group_enabled, group_enforced, group_silent, group_restart_required) =
            compute_effective_values(&ms_config, group_obj, None);

        // Conflict flags: override disagrees with what the group would give
        let override_conflict_enabled = override_val
            .and_then(|o| o.enabled)
            .map(|v| v != group_enabled)
            .unwrap_or(false);
        let override_conflict_enforced = override_val
            .and_then(|o| o.enforced)
            .map(|v| v != group_enforced)
            .unwrap_or(false);
        let override_conflict_silent = override_val
            .and_then(|o| o.silent)
            .map(|v| v != group_silent)
            .unwrap_or(false);
        let override_conflict_restart_required = override_val
            .and_then(|o| o.restart_required)
            .map(|v| v != group_restart_required)
            .unwrap_or(false);

        // Headless warning: per-mod override with enabled=Some(true) + group has exclude_headless=true
        let headless_warning = override_val
            .and_then(|o| o.enabled)
            .map(|enabled| enabled && group_obj.map(|g| g.exclude_headless).unwrap_or(false))
            .unwrap_or(false);

        // Headless disabled: mod is in an exclude_headless group AND no per-mod enabled=Some(true) override
        let headless_disabled = group_obj.map(|g| g.exclude_headless).unwrap_or(false)
            && !override_val.and_then(|o| o.enabled).unwrap_or(false);

        rows.push(ModOverrideRow {
            db_id,
            forge_mod_id: *forge_id,
            name: name.clone(),
            group: group_obj.map(|g| g.display_name.clone()),
            override_enabled: opt_bool_to_val(override_val.and_then(|o| o.enabled)),
            override_enforced: opt_bool_to_val(override_val.and_then(|o| o.enforced)),
            override_silent: opt_bool_to_val(override_val.and_then(|o| o.silent)),
            override_restart_required: opt_bool_to_val(
                override_val.and_then(|o| o.restart_required),
            ),
            effective_enabled: eff_enabled,
            effective_enforced: eff_enforced,
            effective_silent: eff_silent,
            effective_restart_required: eff_restart_required,
            override_conflict_enabled,
            override_conflict_enforced,
            override_conflict_silent,
            override_conflict_restart_required,
            headless_warning,
            headless_disabled,
        });
    }

    // Sort by mod name
    rows.sort_by(|a, b| a.name.cmp(&b.name));

    let tmpl = ModsPartialTemplate {
        csrf_token: csrf_token.to_string(),
        mods: rows,
    };
    tmpl.render().map_err(WebError::from)
}

pub async fn mods_partial(
    state: Data<AppState>,
    req: HttpRequest,
    session: Session,
) -> actix_web::Result<HttpResponse> {
    let user = require_auth(&req)?;
    require_permission(&user, Permission::ModsyncManage)?;
    let csrf_token = crate::web::csrf::get_or_create_token(&session);

    let html = render_mods_tab(&state, &csrf_token).await?;
    Ok(HttpResponse::Ok()
        .content_type("text/html; charset=utf-8")
        .body(html))
}

/// Shared logic: render the preview tab HTML from config + DB state.
async fn render_preview_tab(state: &AppState) -> Result<String, WebError> {
    let ms_config = state.config().modsync.clone().ok_or(WebError::NotFound)?;

    let has_headless_groups = ms_config.groups.values().any(|g| g.exclude_headless);
    let ms_config_clone = ms_config.clone();
    let db = state.db.clone();

    let (player, headless) = web::block(move || {
        let db = db.lock();
        let player = crate::modsync::preview_config(&ms_config_clone, &db, false)?;
        let headless = if has_headless_groups {
            crate::modsync::preview_config(&ms_config_clone, &db, true)?
        } else {
            String::new()
        };
        Ok::<_, anyhow::Error>((player, headless))
    })
    .await
    .map_err(WebError::from)?
    .map_err(WebError::from)?;

    let tmpl = PreviewPartialTemplate {
        player_yaml: player,
        headless_yaml: headless,
        has_headless_groups,
    };
    tmpl.render().map_err(WebError::from)
}

pub async fn save_mods(
    state: Data<AppState>,
    req: HttpRequest,
    session: Session,
    body: Json<serde_json::Value>,
) -> actix_web::Result<HttpResponse> {
    let user = require_auth(&req)?;
    require_permission(&user, Permission::ModsyncManage)?;

    let csrf = body
        .get("csrf_token")
        .and_then(|v| v.as_str())
        .unwrap_or("");
    if !crate::web::csrf::validate_token(&session, csrf) {
        return Err(WebError::Forbidden.into());
    }

    let mods_val = body
        .get("mods")
        .and_then(|v| v.as_array())
        .cloned()
        .unwrap_or_default();

    let mut new_overrides: BTreeMap<String, crate::config::ModSyncOverride> = BTreeMap::new();

    for mod_val in &mods_val {
        let forge_mod_id = mod_val
            .get("forge_mod_id")
            .and_then(|v| v.as_i64())
            .unwrap_or(0);
        if forge_mod_id == 0 {
            continue;
        }

        let enabled = parse_opt_bool(mod_val, "enabled");
        let enforced = parse_opt_bool(mod_val, "enforced");
        let silent = parse_opt_bool(mod_val, "silent");
        let restart_required = parse_opt_bool(mod_val, "restart_required");

        // Only create an override entry if at least one field is non-None
        if enabled.is_some() || enforced.is_some() || silent.is_some() || restart_required.is_some()
        {
            let override_entry = crate::config::ModSyncOverride {
                enabled,
                enforced,
                silent,
                restart_required,
            };
            new_overrides.insert(forge_mod_id.to_string(), override_entry);
        }
    }

    // Load-then-merge: overlay form changes onto existing overrides,
    // preserving entries for mods not in the form (server-only mods, etc.).
    let _guard = state.config_lock.lock();
    let mut config = Config::load(&state.config_path).map_err(WebError::from)?;
    let ms = config
        .modsync
        .get_or_insert_with(crate::config::ModSyncConfig::default);
    for (key, val) in new_overrides {
        if val.enabled.is_none()
            && val.enforced.is_none()
            && val.silent.is_none()
            && val.restart_required.is_none()
        {
            ms.overrides.remove(&key); // All None = remove override entirely
        } else {
            ms.overrides.insert(key, val);
        }
    }
    config.save(&state.config_path).map_err(WebError::from)?;
    if let Err(e) = state.update_config_from_disk() {
        tracing::warn!(err = %e, "failed to refresh in-memory config after save");
    }
    drop(_guard);

    // Regenerate NarcoNet config.yaml
    if state.is_modsync_installed() {
        let db = state.db.clone();
        let spt_dir = state.spt_dir.clone();
        let saved_config = config.clone();
        let _ = web::block(move || {
            let db = db.lock();
            crate::modsync::regenerate_if_enabled(&spt_dir, &saved_config, &db)
        })
        .await;
    }

    set_flash(&session, "NarcoNet overrides saved", FlashType::Success);
    Ok(HttpResponse::Ok().json(serde_json::json!({
        "ok": true,
        "redirect": "/quma/modsync?tab=mods"
    })))
}

// ─── Preview Tab ────────────────────────────────────────────────────────────

#[derive(Template)]
#[template(path = "modsync/partials/preview.html")]
struct PreviewPartialTemplate {
    player_yaml: String,
    headless_yaml: String,
    has_headless_groups: bool,
}

pub async fn preview_partial(
    state: Data<AppState>,
    req: HttpRequest,
    _session: Session,
) -> actix_web::Result<HttpResponse> {
    let user = require_auth(&req)?;
    require_permission(&user, Permission::ModsyncManage)?;

    let ms_config = state.config().modsync.clone().ok_or(WebError::NotFound)?;

    let has_headless_groups = ms_config.groups.values().any(|g| g.exclude_headless);
    let ms_config_clone = ms_config.clone();
    let db = state.db.clone();

    let (player_yaml, headless_yaml) = web::block(move || {
        let db = db.lock();
        let player = crate::modsync::preview_config(&ms_config_clone, &db, false)?;
        let headless = if has_headless_groups {
            crate::modsync::preview_config(&ms_config_clone, &db, true)?
        } else {
            String::new()
        };
        Ok::<_, anyhow::Error>((player, headless))
    })
    .await
    .map_err(WebError::from)?
    .map_err(WebError::from)?;

    let tmpl = PreviewPartialTemplate {
        player_yaml,
        headless_yaml,
        has_headless_groups,
    };
    Ok(HttpResponse::Ok()
        .content_type("text/html; charset=utf-8")
        .body(tmpl.render().map_err(WebError::from)?))
}
