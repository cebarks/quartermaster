use std::collections::BTreeMap;

use actix_session::Session;
use actix_web::web::{self, Data, Form, Json, Query};
use actix_web::{HttpRequest, HttpResponse};
use askama::Template;

use crate::config::{validate_group_slug, Config, ModSyncGroup, NARCONET_FORGE_MOD_ID};
use crate::db::mods::InstalledMod;
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

    // Re-read config from disk to pick up changes saved by save_settings
    // (state.config is immutable behind Arc, so it goes stale after saves)
    let live_config = crate::config::Config::load_with_env(&state.config_path)
        .ok()
        .and_then(|c| c.modsync);
    let ms_config = live_config.or_else(|| state.config.modsync.clone());

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
        "groups" => render_groups_tab(&state, &csrf_token)?,
        "mods" => "<p>Mods tab coming soon</p>".to_string(),
        "preview" => "<p>Preview tab coming soon</p>".to_string(),
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

    let live_config = crate::config::Config::load_with_env(&state.config_path)
        .ok()
        .and_then(|c| c.modsync);
    let ms_config = live_config.or_else(|| state.config.modsync.clone());

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
    // Preserve existing overrides
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

    let ms_config = crate::config::ModSyncConfig {
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

/// Convert an Option<bool> to three-state string for template select.
fn opt_bool_to_val(v: Option<bool>) -> String {
    match v {
        Some(true) => "true".to_string(),
        Some(false) => "false".to_string(),
        None => String::new(),
    }
}

/// Identify which installed mods have at least one BepInEx/ client file.
fn mods_with_client_files(state: &AppState) -> Result<Vec<(i64, String, bool)>, WebError> {
    let db = state.db.lock();
    let mods = db.list_mods().map_err(WebError::from)?;
    let mut result = Vec::new();
    for m in &mods {
        if m.forge_mod_id == NARCONET_FORGE_MOD_ID {
            continue;
        }
        let files = db.get_files_for_mod(m.id).map_err(WebError::from)?;
        let has_client = files.iter().any(|f| f.file_path.starts_with("BepInEx/"));
        result.push((m.forge_mod_id, m.name.clone(), has_client));
    }
    Ok(result)
}

/// Shared logic: build the groups tab HTML from config + DB state.
fn render_groups_tab(state: &AppState, csrf_token: &str) -> Result<String, WebError> {
    let live_config = Config::load_with_env(&state.config_path)
        .ok()
        .and_then(|c| c.modsync);
    let ms_config = live_config
        .or_else(|| state.config.modsync.clone())
        .unwrap_or_default();

    let all_mods = mods_with_client_files(state)?;

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

    let html = render_groups_tab(&state, &csrf_token)?;
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

    let all_mods = mods_with_client_files(&state)?;

    // Load current config to know which mods are already assigned
    let live_config = Config::load_with_env(&state.config_path)
        .ok()
        .and_then(|c| c.modsync);
    let ms_config = live_config
        .or_else(|| state.config.modsync.clone())
        .unwrap_or_default();

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
    let all_mods = mods_with_client_files(&state)?;
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
