use actix_session::Session;
use actix_web::web::{self, Data, Form, Html, Path, Query};
use actix_web::{HttpRequest, HttpResponse};
use askama::Template;

use crate::db::addons::InstalledAddon;
use crate::db::mods::{
    InstalledFile, InstalledMod, ModDependency, ModListFilter, ModSortColumn, ModStatusFilter,
    SortDirection,
};
use crate::db::rbac::Permission;
use crate::db::users::QueueAction;
use crate::forge::models::{DependencyNode, FikaCompat};
use crate::health::{self, IntegrityHealth};
use crate::web::auth::{require_auth, require_permission, SessionUser};
use crate::web::error::WebError;
use crate::web::flash::{set_flash, take_flash, FlashMessage, FlashType};
use crate::web::nav::NavContext;
use crate::web::state::AppState;

#[allow(unused_imports)] // Used by Askama template macro expansion
mod filters {
    pub use crate::web::template_filters::*;
}

// -- Constants --

use crate::config::{FIKA_CLIENT_FORGE_ID, FIKA_SERVER_FORGE_ID};

const INFRASTRUCTURE_FORGE_IDS: &[i64] = &[FIKA_CLIENT_FORGE_ID, FIKA_SERVER_FORGE_ID];

fn is_infrastructure_mod(forge_mod_id: i64) -> bool {
    INFRASTRUCTURE_FORGE_IDS.contains(&forge_mod_id)
}

fn parse_mod_list_query(query: &ModListQuery) -> ModListFilter {
    let search = query
        .q
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(String::from);

    let status = match query.status.as_deref() {
        Some("enabled") => Some(ModStatusFilter::Enabled),
        Some("disabled") => Some(ModStatusFilter::Disabled),
        _ => None,
    };

    let sort_column = match query.sort.as_deref() {
        Some("version") => ModSortColumn::Version,
        Some("files") => ModSortColumn::Files,
        Some("size") => ModSortColumn::Size,
        Some("installed") => ModSortColumn::Installed,
        _ => ModSortColumn::Name,
    };

    let sort_dir = match query.dir.as_deref() {
        Some("desc") => SortDirection::Desc,
        _ => SortDirection::Asc,
    };

    ModListFilter {
        search,
        status,
        sort_column,
        sort_dir,
    }
}

fn sort_column_str(col: ModSortColumn) -> &'static str {
    match col {
        ModSortColumn::Name => "name",
        ModSortColumn::Version => "version",
        ModSortColumn::Files => "files",
        ModSortColumn::Size => "size",
        ModSortColumn::Installed => "installed",
    }
}

fn sort_dir_str(dir: SortDirection) -> &'static str {
    match dir {
        SortDirection::Asc => "asc",
        SortDirection::Desc => "desc",
    }
}

// -- View models --

struct ModListEntry {
    mod_info: InstalledMod,
    file_count: usize,
    total_size: i64,
    addon_count: usize,
    has_client_files: bool,
    excluded_from_headless: bool,
}

impl ModListEntry {
    fn from_db_row(
        row: (InstalledMod, usize, i64),
        addon_counts: &std::collections::HashMap<i64, usize>,
        client_file_mods: &std::collections::HashSet<i64>,
        excluded_mods: &std::collections::HashSet<i64>,
    ) -> Self {
        let (mod_info, file_count, total_size) = row;
        let addon_count = addon_counts.get(&mod_info.id).copied().unwrap_or(0);
        let has_client_files = client_file_mods.contains(&mod_info.id);
        let excluded_from_headless = mod_info
            .forge_mod_id
            .is_some_and(|id| excluded_mods.contains(&id));
        ModListEntry {
            mod_info,
            file_count,
            total_size,
            addon_count,
            has_client_files,
            excluded_from_headless,
        }
    }
}

struct DepEntry {
    dep: ModDependency,
    dep_mod: Option<InstalledMod>,
}

// -- Templates --

#[derive(Template)]
#[template(path = "mods/list.html")]
struct ModListTemplate {
    user: SessionUser,
    infrastructure: Vec<ModListEntry>,
    nav: NavContext,
    mods: Vec<ModListEntry>,
    grand_total_size: i64,
    spt_version: String,
    tarkov_version: String,
    flash: Option<FlashMessage>,
    csrf_token: String,
    filter_q: String,
    filter_status: String,
    sort_column: String,
    sort_dir: String,
    has_any_mods: bool,
}

#[derive(Template)]
#[template(path = "mods/detail.html")]
struct ModDetailTemplate {
    user: SessionUser,
    mod_info: InstalledMod,
    archive_files: Vec<InstalledFile>,
    runtime_files: Vec<InstalledFile>,
    dependencies: Vec<DepEntry>,
    addons: Vec<InstalledAddon>,
    /// Pre-computed permission flags for the addon table partial.
    can_disable: bool,
    can_remove: bool,
    flash: Option<FlashMessage>,
    csrf_token: String,
    nav: NavContext,
    has_client_files: bool,
    sync_enforced: Option<bool>,
    sync_silent: Option<bool>,
    sync_restart_required: Option<bool>,
    sync_enabled: Option<bool>,
    modsync_managed: bool,
}

#[derive(Template)]
#[template(path = "mods/partials/update_badges.html")]
struct UpdateBadgesTemplate {
    updates_available: usize,
}

#[derive(Template)]
#[template(path = "mods/partials/dependency_tree.html")]
struct DependencyTreeTemplate {
    deps: Vec<DependencyNode>,
}

struct UpdateStatusEntry {
    db_id: i64,
    installed_version: String,
    new_version: Option<String>,
    csrf_token: String,
}

#[derive(Template)]
#[template(path = "mods/partials/update_status.html")]
struct UpdateStatusTemplate {
    entries: Vec<UpdateStatusEntry>,
}

struct UpdatesCarouselEntry {
    db_id: i64,
    forge_mod_id: i64,
    name: String,
    slug: Option<String>,
    current_version: String,
    new_version: String,
    update_reason: String,
    spt_version: Option<String>,
    fika_compat: Option<String>,
    download_size: Option<i64>,
    csrf_token: String,
}

#[derive(Template)]
#[template(path = "mods/partials/updates_carousel.html")]
struct UpdatesCarouselTemplate {
    user: SessionUser,
    entry: Option<UpdatesCarouselEntry>,
    total: usize,
    index: usize,
    prev_index: usize,
    next_index: usize,
    csrf_token: String,
}

#[derive(serde::Deserialize)]
pub struct CarouselQuery {
    index: Option<usize>,
}

#[derive(Template)]
#[template(path = "mods/partials/list_body.html")]
struct ListBodyTemplate {
    user: SessionUser,
    mods: Vec<ModListEntry>,
    grand_total_size: i64,
    csrf_token: String,
    sort_column: String,
    sort_dir: String,
}

// -- Form structs --

#[derive(serde::Deserialize)]
pub struct InstallForm {
    mod_ref: String,
    csrf_token: String,
}

#[derive(serde::Deserialize)]
pub struct ModSearchQuery {
    pub q: Option<String>,
}

#[derive(serde::Deserialize)]
pub struct ModListQuery {
    pub q: Option<String>,
    pub status: Option<String>,
    pub sort: Option<String>,
    pub dir: Option<String>,
}

#[derive(Template)]
#[template(path = "mods/partials/install_search_results.html")]
struct InstallSearchResultsTemplate {
    results: Vec<crate::web::handlers::common::ForgeSearchResult>,
    error: Option<String>,
}

#[derive(Template)]
#[template(path = "mods/partials/compat_badge.html")]
struct CompatBadgeTemplate {
    status: String,
}

#[derive(serde::Deserialize)]
pub struct DepTreeQuery {
    #[serde(rename = "mod")]
    mod_id: Option<i64>,
    ver: Option<String>,
    ver_id: Option<i64>,
}

// -- Helpers --

fn empty_carousel(user: SessionUser, csrf_token: String) -> UpdatesCarouselTemplate {
    UpdatesCarouselTemplate {
        user,
        entry: None,
        total: 0,
        index: 0,
        prev_index: 0,
        next_index: 0,
        csrf_token,
    }
}

/// Fetch installed mods from the DB (async-safe via web::block).
async fn list_installed_mods(
    db: std::sync::Arc<parking_lot::Mutex<crate::db::Database>>,
) -> Result<Vec<InstalledMod>, WebError> {
    web::block(move || {
        let db = db.lock();
        db.list_mods()
    })
    .await
    .map_err(WebError::from)?
    .map_err(WebError::from)
}

/// Fetch updates data from cache or Forge API.
async fn get_or_fetch_updates(
    state: &Data<AppState>,
    installed: &[InstalledMod],
) -> Option<crate::forge::models::UpdatesResponseData> {
    if let Some(cached) = state.update_cache.get() {
        return Some(cached);
    }
    let check_list: Vec<(i64, String)> = installed
        .iter()
        .filter_map(|m| m.forge_mod_id.map(|id| (id, m.version.clone())))
        .collect();
    match state
        .forge
        .check_updates(&check_list, &state.spt_info.spt_version)
        .await
    {
        Ok(data) => {
            state.update_cache.set(data.clone());
            Some(data)
        }
        Err(_) => None,
    }
}

/// Check if a mod operation should be queued (server running + queue enabled).
async fn should_queue_operation(state: &Data<AppState>) -> bool {
    let config = state.config_cloned();
    crate::queue::should_queue(
        &config,
        false,
        &state.spt_dir,
        state.container_mgr.as_deref(),
    )
    .await
    .unwrap_or(false)
}

/// Try to queue a mod operation. Returns Ok(Some(response)) if queued (caller should return),
/// Ok(None) if not queued (caller should proceed with immediate operation).
async fn try_queue_mod_op(
    state: &Data<AppState>,
    session: &Session,
    action: QueueAction,
    forge_mod_id: i64,
    version_id: Option<i64>,
    mod_name: &str,
    redirect_url: &str,
) -> Result<Option<HttpResponse>, WebError> {
    if !should_queue_operation(state).await {
        return Ok(None);
    }
    let db = state.db.clone();
    let action_clone = action;
    let mod_name_owned = mod_name.to_string();
    let already_queued = web::block(move || {
        let db = db.lock();
        if db.has_pending_op(forge_mod_id, action_clone)? {
            return Ok::<bool, rusqlite::Error>(true);
        }
        db.insert_pending_op(
            action_clone,
            forge_mod_id,
            version_id,
            &mod_name_owned,
            None,
            None,
        )?;
        Ok(false)
    })
    .await
    .map_err(WebError::from)?
    .map_err(WebError::from)?;

    let action_str = match action {
        QueueAction::Install => "install",
        QueueAction::Update => "update",
        QueueAction::Remove => "removal",
    };
    if already_queued {
        set_flash(
            session,
            &format!("This mod is already queued for {action_str}"),
            FlashType::Warning,
        );
    } else {
        let past = match action {
            QueueAction::Install => "Mod queued for install",
            QueueAction::Update => "Update queued",
            QueueAction::Remove => "Mod queued for removal",
        };
        set_flash(session, past, FlashType::Success);
    }
    Ok(Some(
        HttpResponse::SeeOther()
            .insert_header(("Location", redirect_url))
            .finish(),
    ))
}

/// Try to queue an addon operation. Returns Ok(Some(response)) if queued,
/// Ok(None) if not queued.
#[allow(clippy::too_many_arguments)]
async fn try_queue_addon_op(
    state: &Data<AppState>,
    session: &Session,
    user: &SessionUser,
    action: QueueAction,
    forge_addon_id: i64,
    version_id: Option<i64>,
    addon_name: &str,
    redirect_url: &str,
) -> Result<Option<HttpResponse>, WebError> {
    if !should_queue_operation(state).await {
        return Ok(None);
    }
    let db = state.db.clone();
    let action_clone = action;
    let addon_name_owned = addon_name.to_string();
    let username = user.username.clone();
    let already_queued = web::block(move || {
        let db = db.lock();
        if db.has_pending_addon_op(forge_addon_id, action_clone)? {
            return Ok::<bool, rusqlite::Error>(true);
        }
        db.insert_pending_addon_op(
            action_clone,
            forge_addon_id,
            version_id,
            &addon_name_owned,
            None,
            Some(&username),
        )?;
        Ok(false)
    })
    .await
    .map_err(WebError::from)?
    .map_err(WebError::from)?;

    let action_str = match action {
        QueueAction::Install => "install",
        QueueAction::Update => "update",
        QueueAction::Remove => "removal",
    };
    if already_queued {
        set_flash(
            session,
            &format!("This addon is already queued for {action_str}"),
            FlashType::Warning,
        );
    } else {
        let past = match action {
            QueueAction::Install => "Addon queued for install",
            QueueAction::Update => "Addon queued for update",
            QueueAction::Remove => "Addon queued for removal",
        };
        set_flash(session, past, FlashType::Success);
    }
    Ok(Some(
        HttpResponse::SeeOther()
            .insert_header(("Location", redirect_url))
            .finish(),
    ))
}

// -- Handlers --

pub async fn list_mods(
    state: Data<AppState>,
    req: HttpRequest,
    session: Session,
    query: Query<ModListQuery>,
) -> actix_web::Result<Html> {
    let user = require_auth(&req)?;
    let flash = take_flash(&session);
    let csrf_token = crate::web::csrf::get_or_create_token(&session);
    let filter = parse_mod_list_query(&query);
    let filter_q = query.q.clone().unwrap_or_default();
    let filter_status = query.status.clone().unwrap_or_else(|| "all".to_string());
    let sc = sort_column_str(filter.sort_column).to_string();
    let sd = sort_dir_str(filter.sort_dir).to_string();
    let db = state.db.clone();

    let (all_unfiltered, filtered_entries, addon_counts, all_files) = web::block(move || {
        let db = db.lock();
        let all = db.list_mods_with_file_counts()?;
        let filtered = db.list_mods_filtered(&filter)?;
        let addon_counts = db.count_addons_by_mod()?;
        let files = db.get_all_tracked_files()?;
        Ok::<_, anyhow::Error>((all, filtered, addon_counts, files))
    })
    .await
    .map_err(WebError::from)?
    .map_err(WebError::from)?;

    // ponytail: build set of mod IDs with client files
    let client_file_mods: std::collections::HashSet<i64> = all_files
        .into_iter()
        .filter(|f| crate::headless_sync::is_client_file(&f.file_path))
        .filter_map(|f| f.mod_id)
        .collect();

    // ponytail: build set of forge mod IDs excluded from headless
    let config = state.config_cloned();
    let excluded_mods: std::collections::HashSet<i64> = all_unfiltered
        .iter()
        .filter(|(m, _, _)| {
            m.forge_mod_id
                .is_some_and(|id| crate::ops::is_excluded_from_headless(&config, id))
        })
        .filter_map(|(m, _, _)| m.forge_mod_id)
        .collect();

    let all_entries: Vec<ModListEntry> = all_unfiltered
        .into_iter()
        .map(|row| ModListEntry::from_db_row(row, &addon_counts, &client_file_mods, &excluded_mods))
        .collect();

    let (infrastructure, non_infra): (Vec<_>, Vec<_>) = all_entries
        .into_iter()
        .partition(|e| e.mod_info.forge_mod_id.is_some_and(is_infrastructure_mod));

    let has_any_mods = !non_infra.is_empty();

    let mods: Vec<ModListEntry> = filtered_entries
        .into_iter()
        .map(|row| ModListEntry::from_db_row(row, &addon_counts, &client_file_mods, &excluded_mods))
        .filter(|e| !e.mod_info.forge_mod_id.is_some_and(is_infrastructure_mod))
        .collect();

    let grand_total_size: i64 = mods.iter().map(|m| m.total_size).sum();

    let tmpl = ModListTemplate {
        user,
        infrastructure,
        nav: NavContext::from_state(&state),
        mods,
        grand_total_size,
        spt_version: state.spt_info.spt_version.clone(),
        tarkov_version: state.spt_info.tarkov_version.clone(),
        flash,
        csrf_token,
        filter_q,
        filter_status,
        sort_column: sc,
        sort_dir: sd,
        has_any_mods,
    };
    Ok(Html::new(tmpl.render().map_err(WebError::from)?))
}

pub async fn mod_detail(
    state: Data<AppState>,
    req: HttpRequest,
    session: Session,
    path: Path<i64>,
) -> actix_web::Result<Html> {
    let user = require_auth(&req)?;
    let flash = take_flash(&session);
    let csrf_token = crate::web::csrf::get_or_create_token(&session);
    let mod_id = path.into_inner();
    let db = state.db.clone();

    let (mod_info, archive_files, runtime_files, dependencies, addons) = web::block(move || {
        let db = db.lock();
        let mod_info = db
            .get_mod(mod_id)?
            .ok_or_else(|| anyhow::anyhow!("mod not found"))?;
        let all_files = db.get_files_for_mod(mod_id)?;
        let (archive_files, runtime_files): (Vec<_>, Vec<_>) =
            all_files.into_iter().partition(|f| f.source != "runtime");
        let deps = db.get_dependencies(mod_id)?;
        let mut dep_entries = Vec::new();
        for dep in deps {
            let dep_mod = db.get_mod(dep.depends_on_mod_id)?;
            dep_entries.push(DepEntry { dep, dep_mod });
        }
        let addons = db.list_addons_for_mod(mod_id)?;
        Ok::<_, anyhow::Error>((mod_info, archive_files, runtime_files, dep_entries, addons))
    })
    .await
    .map_err(WebError::from)?
    .map_err(WebError::from)?;

    let has_client_files = archive_files
        .iter()
        .any(|f| f.file_path.starts_with("BepInEx/"));

    let nav = NavContext::from_state(&state);
    let modsync_managed = nav.modsync_installed && nav.modsync_enabled;
    let can_disable = user.can("mods.disable");
    let can_remove = user.can("mods.remove");
    let tmpl = ModDetailTemplate {
        user,
        mod_info,
        archive_files,
        runtime_files,
        dependencies,
        addons,
        can_disable,
        can_remove,
        flash,
        csrf_token,
        nav,
        has_client_files,
        sync_enforced: None,
        sync_silent: None,
        sync_restart_required: None,
        sync_enabled: None,
        modsync_managed,
    };
    Ok(Html::new(tmpl.render().map_err(WebError::from)?))
}

pub async fn check_updates_partial(
    state: Data<AppState>,
    req: HttpRequest,
) -> actix_web::Result<Html> {
    let _user = require_auth(&req)?;
    // No capability check — dashboard shows update badges to all users.
    let installed = list_installed_mods(state.db.clone()).await?;
    let installed: Vec<InstalledMod> = if state.config_cloned().update_disabled_mods {
        installed
    } else {
        installed.into_iter().filter(|m| !m.disabled).collect()
    };

    let updates_available = if !installed.is_empty() {
        let updates_data = match get_or_fetch_updates(&state, &installed).await {
            Some(data) => data,
            None => {
                let tmpl = UpdateBadgesTemplate {
                    updates_available: 0,
                };
                return Ok(Html::new(tmpl.render().map_err(WebError::from)?));
            }
        };

        let mods_with_candidates: Vec<_> = installed
            .iter()
            .filter(|m| {
                updates_data.updates.iter().any(|u| {
                    m.forge_mod_id == Some(u.current_version.mod_id)
                        && u.recommended_version.version != m.version
                })
            })
            .collect();

        let version_futures = mods_with_candidates.iter().filter_map(|m| {
            m.forge_mod_id.map(|forge_id| {
                state
                    .forge
                    .get_versions(forge_id, Some(&state.spt_info.spt_version))
            })
        });
        let results = futures_util::future::join_all(version_futures).await;

        mods_with_candidates
            .iter()
            .zip(results)
            .filter(|(m, result)| {
                result
                    .as_ref()
                    .ok()
                    .and_then(|versions| versions.iter().max_by_key(|v| v.id))
                    .map(|v| &v.version)
                    .is_some_and(|v| v != &m.version)
            })
            .count()
    } else {
        0
    };

    let tmpl = UpdateBadgesTemplate { updates_available };
    Ok(Html::new(tmpl.render().map_err(WebError::from)?))
}

pub async fn update_status_partial(
    state: Data<AppState>,
    req: HttpRequest,
    session: Session,
) -> actix_web::Result<Html> {
    let _user = require_auth(&req)?;
    // No capability check — the OOB swap targets only exist in admin columns,
    // so the response is silently ignored for Players.
    let csrf_token = crate::web::csrf::get_or_create_token(&session);

    let installed = list_installed_mods(state.db.clone()).await?;
    let installed: Vec<InstalledMod> = if state.config_cloned().update_disabled_mods {
        installed
    } else {
        installed.into_iter().filter(|m| !m.disabled).collect()
    };

    if installed.is_empty() {
        let tmpl = UpdateStatusTemplate { entries: vec![] };
        return Ok(Html::new(tmpl.render().map_err(WebError::from)?));
    }

    let updates_data = match get_or_fetch_updates(&state, &installed).await {
        Some(data) => data,
        None => {
            let entries = installed
                .iter()
                .map(|m| UpdateStatusEntry {
                    db_id: m.id,
                    installed_version: m.version.clone(),
                    new_version: None,
                    csrf_token: csrf_token.clone(),
                })
                .collect();
            let tmpl = UpdateStatusTemplate { entries };
            return Ok(Html::new(tmpl.render().map_err(WebError::from)?));
        }
    };

    let needs_check: Vec<usize> = installed
        .iter()
        .enumerate()
        .filter(|(_, m)| {
            updates_data.updates.iter().any(|u| {
                m.forge_mod_id == Some(u.current_version.mod_id)
                    && u.recommended_version.version != m.version
            })
        })
        .map(|(i, _)| i)
        .collect();

    let version_futures = needs_check.iter().filter_map(|&i| {
        let m = &installed[i];
        m.forge_mod_id.map(|forge_id| {
            state
                .forge
                .get_versions(forge_id, Some(&state.spt_info.spt_version))
        })
    });
    let version_results = futures_util::future::join_all(version_futures).await;

    let mut version_map: std::collections::HashMap<usize, Option<String>> =
        std::collections::HashMap::new();
    for (&idx, result) in needs_check.iter().zip(version_results) {
        let new_ver = result
            .ok()
            .and_then(|versions| {
                versions
                    .iter()
                    .max_by_key(|v| v.id)
                    .map(|v| v.version.clone())
            })
            .filter(|v| v != &installed[idx].version);
        version_map.insert(idx, new_ver);
    }

    let entries: Vec<_> = installed
        .iter()
        .enumerate()
        .map(|(i, m)| UpdateStatusEntry {
            db_id: m.id,
            installed_version: m.version.clone(),
            new_version: version_map.get(&i).and_then(|v| v.clone()),
            csrf_token: csrf_token.clone(),
        })
        .collect();

    let tmpl = UpdateStatusTemplate { entries };
    Ok(Html::new(tmpl.render().map_err(WebError::from)?))
}

pub async fn updates_carousel_partial(
    state: Data<AppState>,
    req: HttpRequest,
    session: Session,
    query: Query<CarouselQuery>,
) -> actix_web::Result<Html> {
    let user = require_auth(&req)?;
    let csrf_token = crate::web::csrf::get_or_create_token(&session);
    let index = query.index.unwrap_or(0);

    let installed = list_installed_mods(state.db.clone()).await?;
    let installed: Vec<InstalledMod> = if state.config_cloned().update_disabled_mods {
        installed
    } else {
        installed.into_iter().filter(|m| !m.disabled).collect()
    };

    if installed.is_empty() {
        let tmpl = empty_carousel(user, csrf_token);
        return Ok(Html::new(tmpl.render().map_err(WebError::from)?));
    }

    let updates_data = match get_or_fetch_updates(&state, &installed).await {
        Some(data) => data,
        None => {
            let tmpl = empty_carousel(user, csrf_token);
            return Ok(Html::new(tmpl.render().map_err(WebError::from)?));
        }
    };

    // Match update entries to installed mods, filtering to those with real updates
    let mut updatable: Vec<(
        &crate::db::mods::InstalledMod,
        &crate::forge::models::UpdateEntry,
    )> = installed
        .iter()
        .filter_map(|m| {
            updates_data
                .updates
                .iter()
                .find(|u| {
                    m.forge_mod_id == Some(u.current_version.mod_id)
                        && u.recommended_version.version != m.version
                })
                .map(|u| (m, u))
        })
        .collect();
    updatable.sort_by_key(|a| a.0.name.to_lowercase());

    let total = updatable.len();

    if total == 0 {
        let tmpl = empty_carousel(user, csrf_token);
        return Ok(Html::new(tmpl.render().map_err(WebError::from)?));
    }

    let clamped_index = index % total;
    let (m, u) = updatable[clamped_index];

    // Fika compat is already on the cached UpdateRecommendedVersion;
    // only call get_versions for the SPT version constraint.
    let fika_compat = u
        .recommended_version
        .fika_compatibility
        .as_ref()
        .map(|f| match f {
            FikaCompat::Compatible => "compatible".to_string(),
            FikaCompat::Incompatible => "incompatible".to_string(),
            FikaCompat::Unknown => "unknown".to_string(),
        });

    let forge_mod_id = m
        .forge_mod_id
        .ok_or(WebError::BadRequest("Mod has no Forge ID".to_string()))?;

    let spt_version = match state
        .forge
        .get_versions(forge_mod_id, Some(&state.spt_info.spt_version))
        .await
    {
        Ok(versions) => versions
            .iter()
            .find(|v| v.version == u.recommended_version.version)
            .and_then(|v| v.spt_version.clone()),
        Err(_) => None,
    };

    let entry = UpdatesCarouselEntry {
        db_id: m.id,
        forge_mod_id,
        name: m.name.clone(),
        slug: m.slug.clone(),
        current_version: m.version.clone(),
        new_version: u.recommended_version.version.clone(),
        update_reason: u.update_reason.clone(),
        spt_version,
        fika_compat,
        download_size: u.recommended_version.content_length.map(|s| s as i64),
        csrf_token: csrf_token.clone(),
    };

    let prev_index = if clamped_index == 0 {
        total - 1
    } else {
        clamped_index - 1
    };
    let next_index = if clamped_index + 1 >= total {
        0
    } else {
        clamped_index + 1
    };

    let tmpl = UpdatesCarouselTemplate {
        user,
        entry: Some(entry),
        total,
        index: clamped_index,
        prev_index,
        next_index,
        csrf_token,
    };
    Ok(Html::new(tmpl.render().map_err(WebError::from)?))
}

pub async fn dep_tree_partial(
    state: Data<AppState>,
    query: Query<DepTreeQuery>,
    req: HttpRequest,
) -> actix_web::Result<Html> {
    let user = require_auth(&req)?;
    require_permission(&user, Permission::ModsInstall)?;

    let empty = || {
        let tmpl = DependencyTreeTemplate { deps: vec![] };
        Ok(Html::new(tmpl.render().map_err(WebError::from)?))
    };

    let mod_id = match query.mod_id {
        Some(id) => id,
        None => return empty(),
    };

    let ver = match (&query.ver, query.ver_id) {
        (Some(v), _) => v.clone(),
        (None, Some(ver_id)) => {
            let versions = match state.forge.get_versions(mod_id, None).await {
                Ok(v) => v,
                Err(_) => return empty(),
            };
            match versions.iter().find(|v| v.id == ver_id) {
                Some(v) => v.version.clone(),
                None => return empty(),
            }
        }
        _ => return empty(),
    };

    let deps = state
        .forge
        .get_dependencies(&[(&mod_id.to_string(), &ver)])
        .await
        .unwrap_or_default();

    let tmpl = DependencyTreeTemplate { deps };
    Ok(Html::new(tmpl.render().map_err(WebError::from)?))
}

pub async fn search_mods(
    state: Data<AppState>,
    req: HttpRequest,
    query: Query<ModSearchQuery>,
) -> actix_web::Result<Html> {
    let user = require_auth(&req)?;
    require_permission(&user, Permission::ModsInstall)?;
    let q = query.q.as_deref().unwrap_or("");

    let (results, error) = crate::web::handlers::common::forge_search(&state.forge, q).await;
    let tmpl = InstallSearchResultsTemplate { results, error };
    Ok(Html::new(tmpl.render().map_err(WebError::from)?))
}

pub async fn compat_check(
    state: Data<AppState>,
    req: HttpRequest,
    path: Path<i64>,
) -> actix_web::Result<Html> {
    let user = require_auth(&req)?;
    require_permission(&user, Permission::ModsInstall)?;
    let mod_id = path.into_inner();

    let status = match state
        .forge
        .get_versions(mod_id, Some(&state.spt_info.spt_version))
        .await
    {
        Ok(versions) if !versions.is_empty() => "compatible".to_string(),
        Ok(_) => "incompatible".to_string(),
        Err(_) => "unknown".to_string(),
    };

    let tmpl = CompatBadgeTemplate { status };
    Ok(Html::new(tmpl.render().map_err(WebError::from)?))
}

async fn install_mod_from_url(
    url: &str,
    state: &Data<AppState>,
    session: &Session,
) -> actix_web::Result<HttpResponse> {
    let mod_name = url
        .rsplit('/')
        .next()
        .and_then(|s| s.split('?').next())
        .unwrap_or("unknown-mod")
        .trim_end_matches(".zip")
        .trim_end_matches(".7z")
        .to_string();

    // Check name collision
    {
        let db = state.db.clone();
        let name_check = mod_name.clone();
        let exists = web::block(move || {
            let db = db.lock();
            db.get_mod_by_name_or_slug(&name_check)
        })
        .await
        .map_err(WebError::from)?
        .map_err(WebError::from)?;

        if exists.is_some() {
            set_flash(
                session,
                &format!("A mod named '{mod_name}' is already installed"),
                FlashType::Error,
            );
            return Ok(HttpResponse::SeeOther()
                .insert_header(("Location", "/quma/mods"))
                .finish());
        }
    }

    // Queue if server running
    if should_queue_operation(state).await {
        let queue_dir = state.spt_dir.join(".quartermaster").join("queued");
        let _ = std::fs::create_dir_all(&queue_dir);

        let timestamp = chrono::Utc::now().format("%Y%m%d%H%M%S");
        let extension = if url.ends_with(".7z") { "7z" } else { "zip" };
        let dest = queue_dir.join(format!("{timestamp}-{mod_name}.{extension}"));

        // Download eagerly
        if let Err(e) = state.forge.download_file(url, &dest).await {
            set_flash(
                session,
                &format!("Failed to download: {e}"),
                FlashType::Error,
            );
            return Ok(HttpResponse::SeeOther()
                .insert_header(("Location", "/quma/mods"))
                .finish());
        }

        let db = state.db.clone();
        let mod_name_q = mod_name.clone();
        let dest_str = dest.to_string_lossy().to_string();
        let url_owned = url.to_string();
        let _ = web::block(move || {
            let db = db.lock();
            db.insert_pending_url_op(
                crate::db::users::QueueAction::Install,
                &mod_name_q,
                &dest_str,
                "url",
                Some(&url_owned),
                None,
            )
        })
        .await
        .map_err(WebError::from)?
        .map_err(WebError::from)?;

        set_flash(
            session,
            "Mod queued for install from URL",
            FlashType::Success,
        );
        return Ok(HttpResponse::SeeOther()
            .insert_header(("Location", "/quma/mods"))
            .finish());
    }

    // Direct install via background task
    let forge = state.forge.clone();
    let spt_dir = state.spt_dir.clone();
    let db = state.db.clone();
    let url_owned = url.to_string();
    let mod_name_task = mod_name.clone();
    let update_cache = state.update_cache.clone();
    let mod_zip_cache = state.mod_zip_cache.clone();
    let state_clone = state.clone();

    // ponytail: use 0 as placeholder forge_mod_id for URL installs
    let task_id = match state
        .tasks
        .start_if_not_running("Installing (URL)", &mod_name, 0)
    {
        Some(id) => id,
        None => {
            set_flash(session, "An install is already running", FlashType::Warning);
            return Ok(HttpResponse::SeeOther()
                .insert_header(("Location", "/quma/mods"))
                .finish());
        }
    };
    let tasks = state.tasks.clone();

    tokio::spawn(async move {
        let result = crate::web::install::web_install_from_url(
            &forge,
            &db,
            &spt_dir,
            &url_owned,
            &mod_name_task,
        )
        .await;

        match result {
            Ok(_) => {
                tracing::info!(url = url_owned, "mod installed from URL successfully");
                update_cache.invalidate();
                mod_zip_cache.invalidate();
                state_clone.regenerate_modsync().await;
                tasks.complete(task_id, "Mod installed from URL".to_string());
            }
            Err(e) => {
                tracing::error!(url = url_owned, err = %e, "URL install failed");
                tasks.fail(task_id, format!("Install failed: {e}"));
            }
        }
    });

    set_flash(
        session,
        &format!("Installing {mod_name} from URL..."),
        FlashType::Success,
    );
    Ok(HttpResponse::SeeOther()
        .insert_header(("Location", "/quma/mods"))
        .finish())
}

pub async fn install_mod(
    form: Form<InstallForm>,
    state: Data<AppState>,
    req: HttpRequest,
    session: Session,
) -> actix_web::Result<HttpResponse> {
    let user = require_auth(&req)?;
    require_permission(&user, Permission::ModsInstall)?;
    if !crate::web::csrf::validate_token(&session, &form.csrf_token) {
        return Err(WebError::Forbidden.into());
    }
    let mod_ref = form.mod_ref.trim();

    if mod_ref.is_empty() {
        set_flash(&session, "Mod reference is required", FlashType::Error);
        return Ok(HttpResponse::SeeOther()
            .insert_header(("Location", "/quma/mods"))
            .finish());
    }

    // URL install — skip Forge resolution entirely
    if mod_ref.starts_with("http://") || mod_ref.starts_with("https://") {
        return install_mod_from_url(mod_ref, &state, &session).await;
    }

    let mod_id: i64 = match mod_ref.parse() {
        Ok(id) => id,
        Err(_) => match state.forge.search_mods(mod_ref).await {
            Ok(results) if results.len() == 1 => results[0].id,
            Ok(results) if results.is_empty() => {
                set_flash(
                    &session,
                    &format!("No mods found matching '{mod_ref}'"),
                    FlashType::Error,
                );
                return Ok(HttpResponse::SeeOther()
                    .insert_header(("Location", "/quma/mods"))
                    .finish());
            }
            Ok(_) => {
                set_flash(
                    &session,
                    &format!("Multiple mods match '{mod_ref}' — use a Forge mod ID instead"),
                    FlashType::Error,
                );
                return Ok(HttpResponse::SeeOther()
                    .insert_header(("Location", "/quma/mods"))
                    .finish());
            }
            Err(_) => {
                set_flash(
                    &session,
                    "Failed to search mods. Please try again.",
                    FlashType::Error,
                );
                return Ok(HttpResponse::SeeOther()
                    .insert_header(("Location", "/quma/mods"))
                    .finish());
            }
        },
    };

    let versions = match state
        .forge
        .get_versions(mod_id, Some(&state.spt_info.spt_version))
        .await
    {
        Ok(v) => v,
        Err(e) => {
            tracing::warn!(mod_id, err = %e, "failed to fetch versions");
            set_flash(
                &session,
                "Could not fetch mod versions. Please try again.",
                FlashType::Error,
            );
            return Ok(HttpResponse::SeeOther()
                .insert_header(("Location", "/quma/mods"))
                .finish());
        }
    };

    let version = match versions.iter().max_by_key(|v| v.id) {
        Some(v) => v,
        None => {
            set_flash(
                &session,
                "No compatible version found for current SPT version",
                FlashType::Error,
            );
            return Ok(HttpResponse::SeeOther()
                .insert_header(("Location", "/quma/mods"))
                .finish());
        }
    };

    let mod_info = match state.forge.get_mod(mod_id, false).await {
        Ok(m) => m,
        Err(e) => {
            tracing::warn!(mod_id, err = %e, "failed to fetch mod info");
            set_flash(
                &session,
                "Could not fetch mod info from Forge. Please try again.",
                FlashType::Error,
            );
            return Ok(HttpResponse::SeeOther()
                .insert_header(("Location", "/quma/mods"))
                .finish());
        }
    };

    // Check Fika compatibility if Fika is installed
    {
        let db = state.db.clone();
        let fika_installed = web::block(move || {
            let db = db.lock();
            Ok::<_, anyhow::Error>(db.get_mod_by_forge_id(FIKA_CLIENT_FORGE_ID)?.is_some())
        })
        .await
        .map_err(WebError::from)?
        .map_err(WebError::from)?;

        if fika_installed {
            use crate::forge::models::FikaCompat;
            match &version.fika_compatibility {
                Some(FikaCompat::Incompatible) => {
                    set_flash(
                        &session,
                        &format!(
                            "Warning: {} v{} is marked as Fika INCOMPATIBLE. It may cause issues with multiplayer.",
                            mod_info.name, version.version
                        ),
                        FlashType::Warning,
                    );
                }
                Some(FikaCompat::Unknown) => {
                    set_flash(
                        &session,
                        &format!(
                            "Note: Fika compatibility for {} v{} is unknown.",
                            mod_info.name, version.version
                        ),
                        FlashType::Warning,
                    );
                }
                _ => {}
            }
        }
    }

    // Check if mod is already installed
    {
        let db = state.db.clone();
        let already_installed = web::block(move || {
            let db = db.lock();
            db.get_mod_by_forge_id(mod_id)
        })
        .await
        .map_err(WebError::from)?
        .map_err(WebError::from)?;

        if already_installed.is_some() {
            set_flash(
                &session,
                "This mod is already installed",
                FlashType::Warning,
            );
            return Ok(HttpResponse::SeeOther()
                .insert_header(("Location", "/quma/mods"))
                .finish());
        }
    }

    // Check if the operation should be queued (server running + queue enabled)
    if let Some(resp) = try_queue_mod_op(
        &state,
        &session,
        QueueAction::Install,
        mod_id,
        Some(version.id),
        &mod_info.name,
        "/quma/mods#queue",
    )
    .await?
    {
        return Ok(resp);
    }

    let task_id = match state
        .tasks
        .start_if_not_running("Installing", &mod_info.name, mod_id)
    {
        Some(id) => id,
        None => {
            set_flash(
                &session,
                "This mod is already being installed",
                FlashType::Warning,
            );
            return Ok(HttpResponse::SeeOther()
                .insert_header(("Location", "/quma/mods"))
                .finish());
        }
    };
    let tasks = state.tasks.clone();
    let forge = state.forge.clone();
    let spt_dir = state.spt_dir.clone();
    let db = state.db.clone();
    let db_edges = db.clone();
    let config = state.config_cloned();
    let version = version.clone();
    let mod_name = mod_info.name.clone();
    let mod_slug = mod_info.slug.clone();
    let update_cache = state.update_cache.clone();
    let mod_zip_cache = state.mod_zip_cache.clone();
    let state_clone = state.clone();

    tokio::spawn(async move {
        let result = async {
            // Install dependencies first
            let dep_db_ids = crate::ops::resolve_and_install_deps(
                &forge, &db, &spt_dir, &config, mod_id, &version,
            )
            .await?;

            tasks.update_message(task_id, format!("Downloading {mod_name}…"));

            let db_id = crate::web::install::web_download_extract_and_record(
                &forge,
                &db,
                &spt_dir,
                mod_id,
                &mod_name,
                mod_slug.as_deref(),
                &version,
            )
            .await?;

            // Record dependency edges
            crate::ops::record_dep_edges(&db_edges, db_id, &dep_db_ids);

            // Regenerate NarcoNet config if enabled
            state_clone.regenerate_modsync().await;

            Ok::<_, anyhow::Error>(())
        }
        .await;

        match result {
            Ok(()) => {
                tracing::info!(mod_id, "mod installed successfully");
                update_cache.invalidate();
                mod_zip_cache.invalidate();
                // Re-check NarcoNet detection (installing NarcoNet itself changes this)
                state_clone.modsync_installed.store(
                    crate::config::is_modsync_installed(&spt_dir),
                    std::sync::atomic::Ordering::Relaxed,
                );
                if mod_id == crate::svm::SVM_FORGE_ID {
                    state_clone
                        .svm_installed
                        .store(true, std::sync::atomic::Ordering::Relaxed);
                    if let Some(ref svm_lock) = state_clone.svm {
                        if let Some(mgr) = crate::svm::SvmManager::detect(&spt_dir) {
                            *svm_lock.write() = mgr;
                        }
                    }
                    tracing::info!("SVM installed — config editor reinitialized");
                }
                tasks.complete(task_id, "Mod installed successfully".to_string());
            }
            Err(e) => {
                tracing::error!(mod_id, err = %e, "mod install failed");
                tasks.fail(task_id, format!("Install failed: {e}"));
            }
        }
    });

    Ok(HttpResponse::SeeOther()
        .insert_header(("Location", "/quma/mods"))
        .finish())
}

pub async fn update_mod(
    state: Data<AppState>,
    path: Path<i64>,
    req: HttpRequest,
    session: Session,
    form: Form<crate::web::csrf::CsrfForm>,
) -> actix_web::Result<HttpResponse> {
    let user = require_auth(&req)?;
    require_permission(&user, Permission::ModsUpdate)?;
    if !crate::web::csrf::validate_token(&session, &form.csrf_token) {
        return Err(WebError::Forbidden.into());
    }
    let mod_db_id = path.into_inner();
    let db = state.db.clone();

    let installed = web::block(move || {
        let db = db.lock();
        db.get_mod(mod_db_id)
    })
    .await
    .map_err(WebError::from)?
    .map_err(WebError::from)?
    .ok_or(WebError::NotFound)?;

    let forge_mod_id = installed
        .forge_mod_id
        .ok_or(WebError::BadRequest("Mod has no Forge ID".to_string()))?;

    let versions = state
        .forge
        .get_versions(forge_mod_id, Some(&state.spt_info.spt_version))
        .await
        .map_err(WebError::from)?;

    let version = versions
        .iter()
        .max_by_key(|v| v.id)
        .ok_or(WebError::BadRequest(
            "No compatible update found".to_string(),
        ))?;

    if version.version == installed.version {
        set_flash(&session, "Already up to date", FlashType::Warning);
        return Ok(HttpResponse::SeeOther()
            .insert_header(("Location", format!("/quma/mods/{mod_db_id}")))
            .finish());
    }

    // Check if the operation should be queued
    if let Some(resp) = try_queue_mod_op(
        &state,
        &session,
        QueueAction::Update,
        forge_mod_id,
        Some(version.id),
        &installed.name,
        "/quma/mods#queue",
    )
    .await?
    {
        return Ok(resp);
    }

    let task_id = match state
        .tasks
        .start_if_not_running("Updating", &installed.name, forge_mod_id)
    {
        Some(id) => id,
        None => {
            set_flash(
                &session,
                "This mod is already being updated",
                FlashType::Warning,
            );
            return Ok(HttpResponse::SeeOther()
                .insert_header(("Location", format!("/quma/mods/{mod_db_id}")))
                .finish());
        }
    };
    let tasks = state.tasks.clone();
    let forge = state.forge.clone();
    let spt_dir = state.spt_dir.clone();
    let db = state.db.clone();
    let config = state.config_cloned();
    let version = version.clone();
    let update_cache = state.update_cache.clone();
    let mod_zip_cache = state.mod_zip_cache.clone();
    let state_clone = state.clone();

    tokio::spawn(async move {
        let result = async {
            let dep_db_ids = crate::ops::resolve_and_install_deps(
                &forge,
                &db,
                &spt_dir,
                &config,
                forge_mod_id,
                &version,
            )
            .await?;

            tasks.update_message(task_id, "Downloading update…".to_string());

            let link = version
                .link
                .as_deref()
                .ok_or_else(|| anyhow::anyhow!("version has no download link"))?;
            let tmp_dir = tempfile::tempdir()?;
            let archive_path = tmp_dir.path().join("mod.zip");
            forge.download_file(link, &archive_path).await?;

            tasks.update_message(task_id, "Extracting update…".to_string());

            // Extract to staging on the same filesystem so rename() works
            let staging_dir = crate::ops::staging_tempdir(&spt_dir)?;
            let staging_path = staging_dir.path().to_path_buf();
            let archive = archive_path.clone();
            let extracted = actix_web::web::block(move || {
                crate::spt::mods::extract_mod(&archive, &staging_path)
            })
            .await??;

            crate::ops::apply_mod_update(
                db.clone(),
                spt_dir.clone(),
                config.clone(),
                staging_dir.path().to_path_buf(),
                extracted,
                mod_db_id,
                version.id,
                version.version.clone(),
            )
            .await?;

            crate::ops::record_dep_edges(&db, mod_db_id, &dep_db_ids);

            // Regenerate NarcoNet config if enabled
            state_clone.regenerate_modsync().await;

            Ok::<_, anyhow::Error>(())
        }
        .await;

        match result {
            Ok(()) => {
                tracing::info!(mod_db_id, "mod updated successfully");
                update_cache.invalidate();
                mod_zip_cache.invalidate();
                tasks.complete(task_id, "Mod updated successfully".to_string());
            }
            Err(e) => {
                tracing::error!(mod_db_id, err = %e, "mod update failed");
                tasks.fail(task_id, format!("Update failed: {e}"));
            }
        }
    });

    Ok(HttpResponse::SeeOther()
        .insert_header(("Location", format!("/quma/mods/{mod_db_id}")))
        .finish())
}

pub async fn remove_mod(
    state: Data<AppState>,
    path: Path<i64>,
    req: HttpRequest,
    session: Session,
    form: Form<crate::web::csrf::CsrfForm>,
) -> actix_web::Result<HttpResponse> {
    let user = require_auth(&req)?;
    require_permission(&user, Permission::ModsRemove)?;
    if !crate::web::csrf::validate_token(&session, &form.csrf_token) {
        return Err(WebError::Forbidden.into());
    }
    let mod_db_id = path.into_inner();

    // Look up the installed mod for queue metadata
    let db = state.db.clone();
    let installed = web::block(move || {
        let db = db.lock();
        db.get_mod(mod_db_id)
    })
    .await
    .map_err(WebError::from)?
    .map_err(WebError::from)?
    .ok_or(WebError::NotFound)?;

    // Check if the operation should be queued (only for forge mods)
    if let Some(forge_mod_id) = installed.forge_mod_id {
        if let Some(resp) = try_queue_mod_op(
            &state,
            &session,
            QueueAction::Remove,
            forge_mod_id,
            None,
            &installed.name,
            "/quma/mods#queue",
        )
        .await?
        {
            return Ok(resp);
        }
    }

    let spt_dir = state.spt_dir.clone();
    let db = state.db.clone();
    let config = state.config_cloned();

    tracing::info!(mod_db_id, mod_name = %installed.name, "removing mod");
    web::block(move || {
        let db = db.lock();
        crate::ops::remove_mod_by_id(&db, &spt_dir, &config, mod_db_id)
    })
    .await
    .map_err(WebError::from)?
    .map_err(WebError::from)?;

    state.update_cache.invalidate();
    state.mod_zip_cache.invalidate();
    // Re-check NarcoNet detection (removing NarcoNet itself changes this)
    state.modsync_installed.store(
        crate::config::is_modsync_installed(&state.spt_dir),
        std::sync::atomic::Ordering::Relaxed,
    );
    if installed.forge_mod_id == Some(crate::svm::SVM_FORGE_ID) {
        state
            .svm_installed
            .store(false, std::sync::atomic::Ordering::Relaxed);
        tracing::info!("SVM removed — config editor disabled");
    }
    set_flash(&session, "Mod removed", FlashType::Success);
    Ok(HttpResponse::SeeOther()
        .insert_header(("Location", "/quma/mods"))
        .finish())
}

pub async fn toggle_disable(
    state: Data<AppState>,
    path: Path<i64>,
    req: HttpRequest,
    session: Session,
    form: Form<crate::web::csrf::CsrfForm>,
) -> actix_web::Result<HttpResponse> {
    let user = require_auth(&req)?;
    require_permission(&user, Permission::ModsDisable)?;
    if !crate::web::csrf::validate_token(&session, &form.csrf_token) {
        return Err(WebError::Forbidden.into());
    }
    let mod_db_id = path.into_inner();

    let db = state.db.clone();
    let installed = web::block(move || {
        let db = db.lock();
        db.get_mod(mod_db_id)
    })
    .await
    .map_err(WebError::from)?
    .map_err(WebError::from)?
    .ok_or(WebError::NotFound)?;

    let spt_dir = state.spt_dir.clone();
    let db = state.db.clone();
    let config = state.config_cloned();
    let was_disabled = installed.disabled;
    let mod_name = installed.name.clone();

    web::block(move || {
        let db = db.lock();
        if was_disabled {
            crate::ops::enable_mod(&db, &spt_dir, &config, mod_db_id)
        } else {
            crate::ops::disable_mod(&db, &spt_dir, &config, mod_db_id)
        }
    })
    .await
    .map_err(WebError::from)?
    .map_err(WebError::from)?;

    state.mod_zip_cache.invalidate();

    if was_disabled {
        set_flash(
            &session,
            &format!("{mod_name} has been enabled"),
            FlashType::Success,
        );
    } else {
        set_flash(
            &session,
            &format!("{mod_name} has been disabled"),
            FlashType::Success,
        );
    }
    Ok(HttpResponse::SeeOther()
        .insert_header(("Location", format!("/quma/mods/{mod_db_id}")))
        .finish())
}

pub async fn update_all_mods(
    state: Data<AppState>,
    req: HttpRequest,
    session: Session,
    form: Form<crate::web::csrf::CsrfForm>,
) -> actix_web::Result<HttpResponse> {
    let user = require_auth(&req)?;
    require_permission(&user, Permission::ModsUpdate)?;
    if !crate::web::csrf::validate_token(&session, &form.csrf_token) {
        return Err(WebError::Forbidden.into());
    }
    let installed = list_installed_mods(state.db.clone()).await?;
    let installed: Vec<InstalledMod> = if state.config_cloned().update_disabled_mods {
        installed
    } else {
        installed.into_iter().filter(|m| !m.disabled).collect()
    };

    if installed.is_empty() {
        return Ok(HttpResponse::SeeOther()
            .insert_header(("Location", "/quma/mods"))
            .finish());
    }

    let check_list: Vec<(i64, String)> = installed
        .iter()
        .filter_map(|m| m.forge_mod_id.map(|id| (id, m.version.clone())))
        .collect();

    let results = state
        .forge
        .check_updates(&check_list, &state.spt_info.spt_version)
        .await
        .map_err(WebError::from)?;

    // Check if operations should be queued (server running + queue enabled)
    if should_queue_operation(&state).await {
        let db = state.db.clone();
        web::block(move || {
            let db = db.lock();
            for update in &results.updates {
                if !db.has_pending_op(update.current_version.mod_id, QueueAction::Update)? {
                    db.insert_pending_op(
                        QueueAction::Update,
                        update.current_version.mod_id,
                        Some(update.recommended_version.id),
                        &update.current_version.name,
                        None,
                        None,
                    )?;
                }
            }
            Ok::<_, anyhow::Error>(())
        })
        .await
        .map_err(WebError::from)?
        .map_err(WebError::from)?;

        set_flash(&session, "All updates queued", FlashType::Success);
        return Ok(HttpResponse::SeeOther()
            .insert_header(("Location", "/quma/mods#queue"))
            .finish());
    }

    let task_id = match state.tasks.start_if_no_active("Updating", "all mods", 0) {
        Some(id) => id,
        None => {
            set_flash(
                &session,
                "Please wait for current operations to finish before updating all",
                FlashType::Warning,
            );
            return Ok(HttpResponse::SeeOther()
                .insert_header(("Location", "/quma/mods"))
                .finish());
        }
    };
    let tasks = state.tasks.clone();
    let forge = state.forge.clone();
    let spt_dir = state.spt_dir.clone();
    let db = state.db.clone();
    let config = state.config_cloned();
    let installed = installed.clone();
    let update_cache = state.update_cache.clone();
    let mod_zip_cache = state.mod_zip_cache.clone();
    let state_clone = state.clone();

    tokio::spawn(async move {
        let total = results.updates.len();
        let mut success_count = 0;

        for (i, update) in results.updates.iter().enumerate() {
            tasks.update_message(task_id, format!("Updating mod {} of {}...", i + 1, total));

            let link = match &update.recommended_version.link {
                Some(l) => l.clone(),
                None => continue,
            };

            let mod_db = match installed
                .iter()
                .find(|m| m.forge_mod_id == Some(update.current_version.mod_id))
            {
                Some(m) => m,
                None => continue,
            };
            let mod_db_id = mod_db.id;

            let result = async {
                tasks.update_message(
                    task_id,
                    format!("Downloading {} ({}/{})…", mod_db.name, i + 1, total),
                );

                let tmp_dir = tempfile::tempdir()?;
                let archive_path = tmp_dir.path().join("mod.zip");
                forge.download_file(&link, &archive_path).await?;

                tasks.update_message(
                    task_id,
                    format!("Extracting {} ({}/{})…", mod_db.name, i + 1, total),
                );

                let staging_dir = crate::ops::staging_tempdir(&spt_dir)?;
                let staging_path = staging_dir.path().to_path_buf();
                let archive = archive_path.clone();
                let extracted = actix_web::web::block(move || {
                    crate::spt::mods::extract_mod(&archive, &staging_path)
                })
                .await??;

                crate::ops::apply_mod_update(
                    db.clone(),
                    spt_dir.clone(),
                    config.clone(),
                    staging_dir.path().to_path_buf(),
                    extracted,
                    mod_db_id,
                    update.recommended_version.id,
                    update.recommended_version.version.clone(),
                )
                .await?;
                Ok::<_, anyhow::Error>(())
            }
            .await;

            match result {
                Ok(()) => success_count += 1,
                Err(e) => tracing::error!(mod_db_id, err = %e, "update failed during update-all"),
            }
        }

        // Regenerate NarcoNet config after all updates
        state_clone.regenerate_modsync().await;

        update_cache.invalidate();
        mod_zip_cache.invalidate();
        // Re-check NarcoNet detection (updating mods might affect NarcoNet state)
        state_clone.modsync_installed.store(
            crate::config::is_modsync_installed(&spt_dir),
            std::sync::atomic::Ordering::Relaxed,
        );

        if success_count == total {
            tasks.complete(task_id, format!("All {total} mods updated successfully"));
        } else if success_count > 0 {
            tasks.complete(
                task_id,
                format!("{success_count}/{total} mods updated (some failed — check logs)"),
            );
        } else {
            tasks.fail(task_id, format!("All {total} updates failed"));
        }
    });

    Ok(HttpResponse::SeeOther()
        .insert_header(("Location", "/quma/mods"))
        .finish())
}

pub async fn list_body_partial(
    state: Data<AppState>,
    req: HttpRequest,
    session: Session,
    query: Query<ModListQuery>,
) -> actix_web::Result<Html> {
    let user = require_auth(&req)?;
    let csrf_token = crate::web::csrf::get_or_create_token(&session);
    let filter = parse_mod_list_query(&query);
    let sc = sort_column_str(filter.sort_column).to_string();
    let sd = sort_dir_str(filter.sort_dir).to_string();
    let db = state.db.clone();

    let (filtered_entries, addon_counts, all_files, all_mods) = web::block(move || {
        let db = db.lock();
        let filtered = db.list_mods_filtered(&filter)?;
        let addon_counts = db.count_addons_by_mod()?;
        let files = db.get_all_tracked_files()?;
        let mods = db.list_mods_with_file_counts()?;
        Ok::<_, anyhow::Error>((filtered, addon_counts, files, mods))
    })
    .await
    .map_err(WebError::from)?
    .map_err(WebError::from)?;

    let client_file_mods: std::collections::HashSet<i64> = all_files
        .into_iter()
        .filter(|f| crate::headless_sync::is_client_file(&f.file_path))
        .filter_map(|f| f.mod_id)
        .collect();

    let config = state.config_cloned();
    let excluded_mods: std::collections::HashSet<i64> = all_mods
        .iter()
        .filter(|(m, _, _)| {
            m.forge_mod_id
                .is_some_and(|id| crate::ops::is_excluded_from_headless(&config, id))
        })
        .filter_map(|(m, _, _)| m.forge_mod_id)
        .collect();

    let mods: Vec<ModListEntry> = filtered_entries
        .into_iter()
        .map(|row| ModListEntry::from_db_row(row, &addon_counts, &client_file_mods, &excluded_mods))
        .filter(|e| !e.mod_info.forge_mod_id.is_some_and(is_infrastructure_mod))
        .collect();

    let grand_total_size: i64 = mods.iter().map(|m| m.total_size).sum();

    let tmpl = ListBodyTemplate {
        user,
        mods,
        grand_total_size,
        csrf_token,
        sort_column: sc,
        sort_dir: sd,
    };
    Ok(Html::new(tmpl.render().map_err(WebError::from)?))
}

#[derive(Template)]
#[template(path = "partials/status_integrity.html")]
struct IntegrityTemplate {
    report: IntegrityHealth,
}

pub async fn integrity_partial(state: Data<AppState>, req: HttpRequest) -> actix_web::Result<Html> {
    require_auth(&req)?;
    let db = state.db.clone();
    let tracked_files = web::block(move || {
        let db = db.lock();
        db.get_all_enabled_mod_files()
    })
    .await
    .map_err(WebError::from)?
    .map_err(WebError::from)?;

    let spt_dir = state.spt_dir.clone();
    let report = web::block(move || health::check_integrity_from(&tracked_files, &spt_dir))
        .await
        .map_err(WebError::from)?
        .map_err(WebError::from)?;
    let tmpl = IntegrityTemplate { report };
    Ok(Html::new(tmpl.render().map_err(WebError::from)?))
}

#[derive(Template)]
#[template(path = "files.html")]
struct FileTrackingTemplate {
    user: SessionUser,
    nav: NavContext,
    flash: Option<FlashMessage>,
    csrf_token: String,
    report: IntegrityHealth,
}

pub async fn file_tracking_page(
    state: Data<AppState>,
    req: HttpRequest,
    session: Session,
) -> actix_web::Result<Html> {
    let user = require_auth(&req)?;
    require_permission(&user, Permission::ModsInstall)?;
    let flash = take_flash(&session);
    let csrf_token = crate::web::csrf::get_or_create_token(&session);

    let db = state.db.clone();
    let tracked_files = web::block(move || {
        let db = db.lock();
        db.get_all_enabled_mod_files()
    })
    .await
    .map_err(WebError::from)?
    .map_err(WebError::from)?;

    let spt_dir = state.spt_dir.clone();
    let report = web::block(move || health::check_integrity_from(&tracked_files, &spt_dir))
        .await
        .map_err(WebError::from)?
        .map_err(WebError::from)?;

    let tmpl = FileTrackingTemplate {
        user,
        nav: NavContext::from_state(&state),
        flash,
        csrf_token,
        report,
    };
    Ok(Html::new(tmpl.render().map_err(WebError::from)?))
}

// ── Addon Handlers ────────────────────────────────────────────────────

#[derive(Template)]
#[template(path = "mods/partials/addon_list.html")]
struct AddonListTemplate {
    addons: Vec<InstalledAddon>,
    csrf_token: String,
    can_disable: bool,
    can_remove: bool,
}

pub async fn list_addons_partial(
    state: Data<AppState>,
    req: HttpRequest,
    session: Session,
    path: Path<i64>,
) -> actix_web::Result<Html> {
    let user = require_auth(&req)?;
    let csrf_token = crate::web::csrf::get_or_create_token(&session);
    let mod_db_id = path.into_inner();
    let db = state.db.clone();

    let addons = web::block(move || {
        let db = db.lock();
        db.list_addons_for_mod(mod_db_id)
    })
    .await
    .map_err(WebError::from)?
    .map_err(WebError::from)?;

    let can_disable = user.can("mods.disable");
    let can_remove = user.can("mods.remove");

    let tmpl = AddonListTemplate {
        addons,
        csrf_token,
        can_disable,
        can_remove,
    };
    Ok(Html::new(tmpl.render().map_err(WebError::from)?))
}

#[derive(serde::Deserialize)]
pub struct AddonSearchQuery {
    pub q: Option<String>,
    pub parent: i64,
}

#[derive(Template)]
#[template(path = "mods/partials/addon_search_results.html")]
struct AddonSearchResultsTemplate {
    results: Vec<AddonSearchResult>,
    parent_mod_db_id: i64,
    csrf_token: String,
    error: Option<String>,
}

pub struct AddonSearchResult {
    pub id: i64,
    pub name: String,
    pub description: Option<String>,
    #[allow(dead_code)]
    pub mod_id: i64,
}

pub async fn search_addons(
    state: Data<AppState>,
    req: HttpRequest,
    session: Session,
    query: Query<AddonSearchQuery>,
) -> actix_web::Result<Html> {
    let _user = require_auth(&req)?;
    let csrf_token = crate::web::csrf::get_or_create_token(&session);
    let parent_mod_db_id = query.parent;

    let search_query = query.q.as_deref().unwrap_or("").trim();
    if search_query.is_empty() {
        let tmpl = AddonSearchResultsTemplate {
            results: vec![],
            parent_mod_db_id,
            csrf_token,
            error: None,
        };
        return Ok(Html::new(tmpl.render().map_err(WebError::from)?));
    }

    // Get the parent mod to find its forge_mod_id
    let db = state.db.clone();
    let parent_mod = web::block(move || {
        let db = db.lock();
        db.get_mod(parent_mod_db_id)
    })
    .await
    .map_err(WebError::from)?
    .map_err(WebError::from)?;

    let parent_forge_mod_id = match parent_mod.and_then(|m| m.forge_mod_id) {
        Some(id) => id,
        None => {
            let tmpl = AddonSearchResultsTemplate {
                results: vec![],
                parent_mod_db_id,
                csrf_token,
                error: Some("Parent mod not found".to_string()),
            };
            return Ok(Html::new(tmpl.render().map_err(WebError::from)?));
        }
    };

    let results = match state.forge.search_addons(search_query).await {
        Ok(forge_results) => forge_results
            .into_iter()
            .filter(|a| a.mod_id == Some(parent_forge_mod_id))
            .map(|a| AddonSearchResult {
                id: a.id,
                name: a.name,
                description: a.description,
                mod_id: a.mod_id.unwrap_or(0),
            })
            .collect(),
        Err(e) => {
            tracing::warn!(err = %e, "failed to search addons");
            let tmpl = AddonSearchResultsTemplate {
                results: vec![],
                parent_mod_db_id,
                csrf_token,
                error: Some("Search failed. Please try again.".to_string()),
            };
            return Ok(Html::new(tmpl.render().map_err(WebError::from)?));
        }
    };

    let tmpl = AddonSearchResultsTemplate {
        results,
        parent_mod_db_id,
        csrf_token,
        error: None,
    };
    Ok(Html::new(tmpl.render().map_err(WebError::from)?))
}

#[derive(serde::Deserialize)]
pub struct InstallAddonForm {
    addon_id: i64,
    csrf_token: String,
}

pub async fn install_addon(
    form: Form<InstallAddonForm>,
    state: Data<AppState>,
    req: HttpRequest,
    session: Session,
    path: Path<i64>,
) -> actix_web::Result<HttpResponse> {
    let user = require_auth(&req)?;
    require_permission(&user, Permission::ModsInstall)?;
    if !crate::web::csrf::validate_token(&session, &form.csrf_token) {
        return Err(WebError::Forbidden.into());
    }

    let parent_mod_db_id = path.into_inner();
    let addon_forge_id = form.addon_id;

    // Get parent mod
    let db = state.db.clone();
    let parent_mod = web::block(move || {
        let db = db.lock();
        db.get_mod(parent_mod_db_id)
    })
    .await
    .map_err(WebError::from)?
    .map_err(WebError::from)?;

    let _parent_mod = match parent_mod {
        Some(m) => m,
        None => {
            set_flash(&session, "Parent mod not found", FlashType::Error);
            return Ok(HttpResponse::SeeOther()
                .insert_header(("Location", "/quma/mods"))
                .finish());
        }
    };

    // Check if already installed
    {
        let db = state.db.clone();
        let already_installed = web::block(move || {
            let db = db.lock();
            db.get_addon_by_forge_id(addon_forge_id)
        })
        .await
        .map_err(WebError::from)?
        .map_err(WebError::from)?;

        if already_installed.is_some() {
            set_flash(
                &session,
                "This addon is already installed",
                FlashType::Warning,
            );
            return Ok(HttpResponse::SeeOther()
                .insert_header(("Location", format!("/quma/mods/{}", parent_mod_db_id)))
                .finish());
        }
    }

    // Fetch addon info from Forge
    let addon_info = match state.forge.get_addon(addon_forge_id, false).await {
        Ok(a) => a,
        Err(e) => {
            tracing::warn!(addon_id = addon_forge_id, err = %e, "failed to fetch addon info");
            set_flash(
                &session,
                "Could not fetch addon info from Forge. Please try again.",
                FlashType::Error,
            );
            return Ok(HttpResponse::SeeOther()
                .insert_header(("Location", format!("/quma/mods/{}", parent_mod_db_id)))
                .finish());
        }
    };

    // Fetch versions
    let versions = match state.forge.get_addon_versions(addon_forge_id).await {
        Ok(v) => v,
        Err(e) => {
            tracing::warn!(addon_id = addon_forge_id, err = %e, "failed to fetch addon versions");
            set_flash(
                &session,
                "Could not fetch addon versions. Please try again.",
                FlashType::Error,
            );
            return Ok(HttpResponse::SeeOther()
                .insert_header(("Location", format!("/quma/mods/{}", parent_mod_db_id)))
                .finish());
        }
    };

    let version = match versions.iter().max_by_key(|v| v.id) {
        Some(v) => v,
        None => {
            set_flash(
                &session,
                "No versions available for this addon",
                FlashType::Error,
            );
            return Ok(HttpResponse::SeeOther()
                .insert_header(("Location", format!("/quma/mods/{}", parent_mod_db_id)))
                .finish());
        }
    };

    // Check if the operation should be queued
    if let Some(resp) = try_queue_addon_op(
        &state,
        &session,
        &user,
        QueueAction::Install,
        addon_forge_id,
        Some(version.id),
        &addon_info.name,
        &format!("/quma/mods/{}#queue", parent_mod_db_id),
    )
    .await?
    {
        return Ok(resp);
    }

    // Install immediately
    let task_id =
        match state
            .tasks
            .start_if_not_running("Installing addon", &addon_info.name, addon_forge_id)
        {
            Some(id) => id,
            None => {
                set_flash(
                    &session,
                    "This addon is already being installed",
                    FlashType::Warning,
                );
                return Ok(HttpResponse::SeeOther()
                    .insert_header(("Location", format!("/quma/mods/{}", parent_mod_db_id)))
                    .finish());
            }
        };

    let tasks = state.tasks.clone();
    let forge = state.forge.clone();
    let spt_dir = state.spt_dir.clone();
    let db = state.db.clone();
    let config = state.config_cloned();
    let version = version.clone();
    let addon_name = addon_info.name.clone();
    let addon_slug = addon_info.slug.clone();
    let mod_zip_cache = state.mod_zip_cache.clone();

    tokio::spawn(async move {
        let result = async {
            tasks.update_message(task_id, format!("Downloading {addon_name}…"));

            let link = version
                .link
                .as_deref()
                .ok_or_else(|| anyhow::anyhow!("version has no download link"))?;
            let tmp_dir = tempfile::tempdir()?;
            let archive_path = tmp_dir.path().join("addon.zip");
            forge.download_file(link, &archive_path).await?;

            tasks.update_message(task_id, format!("Extracting {addon_name}…"));

            let db_ref = &db.lock();
            let req = crate::ops::InstallAddonRequest {
                db: db_ref,
                spt_dir: &spt_dir,
                config: &config,
                forge_addon_id: Some(addon_forge_id),
                parent_mod_id: parent_mod_db_id,
                version_id: Some(version.id),
                name: &addon_name,
                slug: addon_slug.as_deref(),
                version: &version.version,
                mod_version_constraint: version.mod_version_constraint.as_deref(),
                archive_path: &archive_path,
                source: crate::ops::ModSource::Forge,
                source_url: None,
            };

            crate::ops::install_addon_from_archive(&req)?;
            Ok::<_, anyhow::Error>(())
        }
        .await;

        match result {
            Ok(_) => {
                tasks.complete(task_id, "Addon installed successfully".to_string());
                mod_zip_cache.invalidate();
            }
            Err(e) => {
                tracing::error!(task_id, err = %e, "addon install failed");
                tasks.fail(task_id, format!("Install failed: {e}"));
            }
        }
    });

    set_flash(&session, "Addon installation started", FlashType::Success);
    Ok(HttpResponse::SeeOther()
        .insert_header(("Location", format!("/quma/mods/{}", parent_mod_db_id)))
        .finish())
}

#[derive(serde::Deserialize)]
pub struct AddonActionForm {
    csrf_token: String,
}

pub async fn update_addon(
    form: Form<AddonActionForm>,
    state: Data<AppState>,
    req: HttpRequest,
    session: Session,
    path: Path<i64>,
) -> actix_web::Result<HttpResponse> {
    let user = require_auth(&req)?;
    require_permission(&user, Permission::ModsUpdate)?;
    if !crate::web::csrf::validate_token(&session, &form.csrf_token) {
        return Err(WebError::Forbidden.into());
    }

    let addon_db_id = path.into_inner();

    // Get addon info
    let db = state.db.clone();
    let addon = web::block(move || {
        let db = db.lock();
        db.get_addon(addon_db_id)
    })
    .await
    .map_err(WebError::from)?
    .map_err(WebError::from)?;

    let addon = match addon {
        Some(a) => a,
        None => {
            set_flash(&session, "Addon not found", FlashType::Error);
            return Ok(HttpResponse::SeeOther()
                .insert_header(("Location", "/quma/mods"))
                .finish());
        }
    };

    // Fetch latest version
    let versions = match state.forge.get_addon_versions(addon.forge_addon_id).await {
        Ok(v) => v,
        Err(e) => {
            tracing::warn!(forge_id = addon.forge_addon_id, err = %e, "failed to fetch addon versions");
            set_flash(
                &session,
                "Could not fetch addon versions. Please try again.",
                FlashType::Error,
            );
            return Ok(HttpResponse::SeeOther()
                .insert_header(("Location", format!("/quma/mods/{}", addon.parent_mod_id)))
                .finish());
        }
    };

    let latest_version = match versions.iter().max_by_key(|v| v.id) {
        Some(v) => v,
        None => {
            set_flash(
                &session,
                "No versions available for this addon",
                FlashType::Error,
            );
            return Ok(HttpResponse::SeeOther()
                .insert_header(("Location", format!("/quma/mods/{}", addon.parent_mod_id)))
                .finish());
        }
    };

    if latest_version.version == addon.version {
        set_flash(&session, "Addon is already up to date", FlashType::Info);
        return Ok(HttpResponse::SeeOther()
            .insert_header(("Location", format!("/quma/mods/{}", addon.parent_mod_id)))
            .finish());
    }

    // Check if the operation should be queued
    if let Some(resp) = try_queue_addon_op(
        &state,
        &session,
        &user,
        QueueAction::Update,
        addon.forge_addon_id,
        Some(latest_version.id),
        &addon.name,
        &format!("/quma/mods/{}#queue", addon.parent_mod_id),
    )
    .await?
    {
        return Ok(resp);
    }

    // Update immediately
    let task_id =
        match state
            .tasks
            .start_if_not_running("Updating addon", &addon.name, addon.forge_addon_id)
        {
            Some(id) => id,
            None => {
                set_flash(
                    &session,
                    "This addon is already being updated",
                    FlashType::Warning,
                );
                return Ok(HttpResponse::SeeOther()
                    .insert_header(("Location", format!("/quma/mods/{}", addon.parent_mod_id)))
                    .finish());
            }
        };

    let tasks = state.tasks.clone();
    let forge = state.forge.clone();
    let spt_dir = state.spt_dir.clone();
    let db = state.db.clone();
    let config = state.config_cloned();
    let version = latest_version.clone();
    let addon_name = addon.name.clone();
    let parent_mod_id = addon.parent_mod_id;
    let mod_zip_cache = state.mod_zip_cache.clone();

    tokio::spawn(async move {
        let result = async {
            tasks.update_message(task_id, format!("Downloading {addon_name} update…"));

            let link = version
                .link
                .as_deref()
                .ok_or_else(|| anyhow::anyhow!("version has no download link"))?;
            let tmp_dir = tempfile::tempdir()?;
            let archive_path = tmp_dir.path().join("addon.zip");
            forge.download_file(link, &archive_path).await?;

            tasks.update_message(task_id, format!("Extracting {addon_name} update…"));

            let staging_dir = crate::ops::staging_tempdir(&spt_dir)?;
            let staging_path = staging_dir.path().to_path_buf();
            let archive = archive_path.clone();
            let staging_path_clone = staging_path.clone();
            let extracted = actix_web::web::block(move || {
                crate::spt::mods::extract_mod(&archive, &staging_path_clone)
            })
            .await??;

            crate::ops::apply_addon_update(
                db,
                spt_dir,
                config,
                staging_path,
                extracted,
                addon_db_id,
                version.id,
                version.version.clone(),
                version.mod_version_constraint.clone(),
                addon.forge_addon_id,
            )
            .await?;
            Ok::<_, anyhow::Error>(())
        }
        .await;

        match result {
            Ok(_) => {
                tasks.complete(task_id, "Addon updated successfully".to_string());
                mod_zip_cache.invalidate();
            }
            Err(e) => {
                tracing::error!(task_id, addon = %addon_name, parent_mod_id, err = %e, "addon update failed");
                tasks.fail(task_id, format!("Update failed: {e}"));
            }
        }
    });

    set_flash(&session, "Addon update started", FlashType::Success);
    Ok(HttpResponse::SeeOther()
        .insert_header(("Location", format!("/quma/mods/{}", addon.parent_mod_id)))
        .finish())
}

pub async fn remove_addon(
    form: Form<AddonActionForm>,
    state: Data<AppState>,
    req: HttpRequest,
    session: Session,
    path: Path<i64>,
) -> actix_web::Result<HttpResponse> {
    let user = require_auth(&req)?;
    require_permission(&user, Permission::ModsRemove)?;
    if !crate::web::csrf::validate_token(&session, &form.csrf_token) {
        return Err(WebError::Forbidden.into());
    }

    let addon_db_id = path.into_inner();

    // Get addon info
    let db = state.db.clone();
    let db2 = state.db.clone();
    let addon = web::block(move || {
        let db = db.lock();
        db.get_addon(addon_db_id)
    })
    .await
    .map_err(WebError::from)?
    .map_err(WebError::from)?;

    let addon = match addon {
        Some(a) => a,
        None => {
            set_flash(&session, "Addon not found", FlashType::Error);
            return Ok(HttpResponse::SeeOther()
                .insert_header(("Location", "/quma/mods"))
                .finish());
        }
    };

    let parent_mod_id = addon.parent_mod_id;
    let spt_dir = state.spt_dir.clone();
    let config = state.config_cloned();

    let result = web::block(move || {
        let db = db2.lock();
        crate::ops::remove_addon_by_id(&db, &spt_dir, &config, addon_db_id, false)
    })
    .await
    .map_err(WebError::from)?;

    match result {
        Ok(_) => {
            set_flash(&session, "Addon removed successfully", FlashType::Success);
            state.mod_zip_cache.invalidate();
        }
        Err(e) => {
            tracing::error!(addon_db_id, err = %e, "addon removal failed");
            set_flash(
                &session,
                &format!("Failed to remove addon: {e}"),
                FlashType::Error,
            );
        }
    }

    Ok(HttpResponse::SeeOther()
        .insert_header(("Location", format!("/quma/mods/{}", parent_mod_id)))
        .finish())
}

pub async fn toggle_addon_disable(
    form: Form<AddonActionForm>,
    state: Data<AppState>,
    req: HttpRequest,
    session: Session,
    path: Path<i64>,
) -> actix_web::Result<HttpResponse> {
    let user = require_auth(&req)?;
    require_permission(&user, Permission::ModsDisable)?;
    if !crate::web::csrf::validate_token(&session, &form.csrf_token) {
        return Err(WebError::Forbidden.into());
    }

    let addon_db_id = path.into_inner();

    // Get addon info
    let db = state.db.clone();
    let addon = web::block(move || {
        let db = db.lock();
        db.get_addon(addon_db_id)
    })
    .await
    .map_err(WebError::from)?
    .map_err(WebError::from)?;

    let addon = match addon {
        Some(a) => a,
        None => {
            set_flash(&session, "Addon not found", FlashType::Error);
            return Ok(HttpResponse::SeeOther()
                .insert_header(("Location", "/quma/mods"))
                .finish());
        }
    };

    let parent_mod_id = addon.parent_mod_id;
    let is_disabled = addon.disabled;
    let spt_dir = state.spt_dir.clone();
    let config = state.config_cloned();
    let db2 = state.db.clone();

    let result = if is_disabled {
        web::block(move || {
            let db = db2.lock();
            crate::ops::enable_addon(&db, &spt_dir, &config, addon_db_id)
        })
        .await
        .map_err(WebError::from)?
    } else {
        web::block(move || {
            let db = db2.lock();
            crate::ops::disable_addon(&db, &spt_dir, &config, addon_db_id)
        })
        .await
        .map_err(WebError::from)?
    };

    match result {
        Ok(_) => {
            let msg = if is_disabled {
                "Addon enabled successfully"
            } else {
                "Addon disabled successfully"
            };
            set_flash(&session, msg, FlashType::Success);
            state.mod_zip_cache.invalidate();
        }
        Err(e) => {
            tracing::error!(addon_db_id, err = %e, "addon toggle failed");
            set_flash(
                &session,
                &format!("Failed to toggle addon: {e}"),
                FlashType::Error,
            );
        }
    }

    Ok(HttpResponse::SeeOther()
        .insert_header(("Location", format!("/quma/mods/{}", parent_mod_id)))
        .finish())
}
