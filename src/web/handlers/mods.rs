use actix_session::Session;
use actix_web::web::{self, Data, Form, Html, Path, Query};
use actix_web::{HttpRequest, HttpResponse};
use askama::Template;

use crate::db::mods::{
    InstalledFile, InstalledMod, ModDependency, ModListFilter, ModSortColumn, ModStatusFilter,
    SortDirection,
};
use crate::db::users::Role;
use crate::forge::models::DependencyNode;
use crate::web::auth::{require_auth, require_capability, SessionUser};
use crate::web::error::WebError;
use crate::web::flash::{set_flash, take_flash, FlashMessage};
use crate::web::handlers::requests::{fika_compat_to_string, parse_forge_url};
use crate::web::state::AppState;

#[allow(unused_imports)] // Used by Askama template macro expansion
mod filters {
    pub use crate::web::template_filters::*;
}

// -- Constants --

const INFRASTRUCTURE_FORGE_IDS: &[i64] = &[
    2326, // Project Fika (client)
    2357, // Project Fika - Server
];

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
    modsync_installed: bool,
    #[allow(dead_code)]
    svm_installed: bool,
    mods: Vec<ModListEntry>,
    grand_total_size: i64,
    spt_version: String,
    tarkov_version: String,
    flash: Option<FlashMessage>,
    csrf_token: String,
    #[allow(dead_code)]
    fika_installed: bool,
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
    flash: Option<FlashMessage>,
    csrf_token: String,
    #[allow(dead_code)]
    fika_installed: bool,
    #[allow(dead_code)]
    modsync_installed: bool,
    #[allow(dead_code)]
    svm_installed: bool,
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

pub struct InstallSearchResult {
    pub id: i64,
    pub name: String,
    pub description: Option<String>,
    pub fika_compatible: String,
}

#[derive(Template)]
#[template(path = "mods/partials/install_search_results.html")]
struct InstallSearchResultsTemplate {
    results: Vec<InstallSearchResult>,
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

    let (all_unfiltered, filtered_entries) = web::block(move || {
        let db = db.lock();
        let all = db.list_mods_with_file_counts()?;
        let filtered = db.list_mods_filtered(&filter)?;
        Ok::<_, anyhow::Error>((all, filtered))
    })
    .await
    .map_err(WebError::from)?
    .map_err(WebError::from)?;

    let all_entries: Vec<ModListEntry> = all_unfiltered
        .into_iter()
        .map(|(mod_info, file_count, total_size)| ModListEntry {
            mod_info,
            file_count,
            total_size,
        })
        .collect();

    let (infrastructure, non_infra): (Vec<_>, Vec<_>) = all_entries
        .into_iter()
        .partition(|e| is_infrastructure_mod(e.mod_info.forge_mod_id));

    let has_any_mods = !non_infra.is_empty();

    let mods: Vec<ModListEntry> = filtered_entries
        .into_iter()
        .map(|(mod_info, file_count, total_size)| ModListEntry {
            mod_info,
            file_count,
            total_size,
        })
        .filter(|e| !is_infrastructure_mod(e.mod_info.forge_mod_id))
        .collect();

    let grand_total_size: i64 = mods.iter().map(|m| m.total_size).sum();
    let modsync_installed = state.is_modsync_installed();
    let svm_installed = state.is_svm_installed();

    let tmpl = ModListTemplate {
        user,
        infrastructure,
        modsync_installed,
        svm_installed,
        mods,
        grand_total_size,
        spt_version: state.spt_info.spt_version.clone(),
        tarkov_version: state.spt_info.tarkov_version.clone(),
        flash,
        csrf_token,
        fika_installed: state.fika_installed,
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

    let (mod_info, archive_files, runtime_files, dependencies) = web::block(move || {
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
        Ok::<_, anyhow::Error>((mod_info, archive_files, runtime_files, dep_entries))
    })
    .await
    .map_err(WebError::from)?
    .map_err(WebError::from)?;

    let has_client_files = archive_files
        .iter()
        .any(|f| f.file_path.starts_with("BepInEx/"));
    let forge_id_str = mod_info.forge_mod_id.to_string();
    let overrides = state
        .config
        .modsync
        .as_ref()
        .and_then(|ms| ms.overrides.get(&forge_id_str));

    let tmpl = ModDetailTemplate {
        user,
        mod_info,
        archive_files,
        runtime_files,
        dependencies,
        flash,
        csrf_token,
        fika_installed: state.fika_installed,
        modsync_installed: state.is_modsync_installed(),
        svm_installed: state.is_svm_installed(),
        has_client_files,
        sync_enforced: overrides.and_then(|o| o.enforced),
        sync_silent: overrides.and_then(|o| o.silent),
        sync_restart_required: overrides.and_then(|o| o.restart_required),
        sync_enabled: overrides.and_then(|o| o.enabled),
        modsync_managed: state.is_modsync_installed() && state.config.modsync.is_some(),
    };
    Ok(Html::new(tmpl.render().map_err(WebError::from)?))
}

pub async fn check_updates_partial(
    state: Data<AppState>,
    req: HttpRequest,
) -> actix_web::Result<Html> {
    let _user = require_auth(&req)?;
    // No capability check — dashboard shows update badges to all users.
    let db = state.db.clone();
    let installed = web::block(move || {
        let db = db.lock();
        db.list_mods()
    })
    .await
    .map_err(WebError::from)?
    .map_err(WebError::from)?;

    let updates_available = if !installed.is_empty() {
        let updates_data = if let Some(cached) = state.update_cache.get() {
            cached
        } else {
            let check_list: Vec<(i64, String)> = installed
                .iter()
                .map(|m| (m.forge_mod_id, m.version.clone()))
                .collect();
            match state
                .forge
                .check_updates(&check_list, &state.spt_info.spt_version)
                .await
            {
                Ok(data) => {
                    state.update_cache.set(data.clone());
                    data
                }
                Err(_) => {
                    let tmpl = UpdateBadgesTemplate {
                        updates_available: 0,
                    };
                    return Ok(Html::new(tmpl.render().map_err(WebError::from)?));
                }
            }
        };

        let mut count = 0usize;
        for m in &installed {
            let candidate = updates_data
                .updates
                .iter()
                .find(|u| u.current_version.mod_id == m.forge_mod_id)
                .map(|u| u.recommended_version.version.clone())
                .filter(|v| v != &m.version);

            if candidate.is_some() {
                if let Ok(versions) = state
                    .forge
                    .get_versions(m.forge_mod_id, Some(&state.spt_info.spt_version))
                    .await
                {
                    if versions
                        .last()
                        .map(|v| &v.version)
                        .is_some_and(|v| v != &m.version)
                    {
                        count += 1;
                    }
                }
            }
        }
        count
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

    let db = state.db.clone();
    let installed = web::block(move || {
        let db = db.lock();
        db.list_mods()
    })
    .await
    .map_err(WebError::from)?
    .map_err(WebError::from)?;

    if installed.is_empty() {
        let tmpl = UpdateStatusTemplate { entries: vec![] };
        return Ok(Html::new(tmpl.render().map_err(WebError::from)?));
    }

    let updates_data = if let Some(cached) = state.update_cache.get() {
        cached
    } else {
        let check_list: Vec<(i64, String)> = installed
            .iter()
            .map(|m| (m.forge_mod_id, m.version.clone()))
            .collect();
        match state
            .forge
            .check_updates(&check_list, &state.spt_info.spt_version)
            .await
        {
            Ok(data) => {
                state.update_cache.set(data.clone());
                data
            }
            Err(_) => {
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
        }
    };

    let mut entries = Vec::with_capacity(installed.len());
    for m in &installed {
        let candidate = updates_data
            .updates
            .iter()
            .find(|u| u.current_version.mod_id == m.forge_mod_id)
            .map(|u| u.recommended_version.version.clone())
            .filter(|v| v != &m.version);

        let new_version = if candidate.is_some() {
            match state
                .forge
                .get_versions(m.forge_mod_id, Some(&state.spt_info.spt_version))
                .await
            {
                Ok(versions) => versions
                    .last()
                    .map(|v| v.version.clone())
                    .filter(|v| v != &m.version),
                Err(_) => None,
            }
        } else {
            None
        };

        entries.push(UpdateStatusEntry {
            db_id: m.id,
            installed_version: m.version.clone(),
            new_version,
            csrf_token: csrf_token.clone(),
        });
    }

    let tmpl = UpdateStatusTemplate { entries };
    Ok(Html::new(tmpl.render().map_err(WebError::from)?))
}

pub async fn dep_tree_partial(
    state: Data<AppState>,
    query: Query<DepTreeQuery>,
    req: HttpRequest,
) -> actix_web::Result<Html> {
    let user = require_auth(&req)?;
    require_capability(&user, Role::can_manage_mods)?;
    let deps = match (query.mod_id, &query.ver) {
        (Some(mod_id), Some(ver)) => state
            .forge
            .get_dependencies(&[(mod_id, ver.as_str())])
            .await
            .unwrap_or_default(),
        _ => vec![],
    };

    let tmpl = DependencyTreeTemplate { deps };
    Ok(Html::new(tmpl.render().map_err(WebError::from)?))
}

pub async fn search_mods(
    state: Data<AppState>,
    req: HttpRequest,
    query: Query<ModSearchQuery>,
) -> actix_web::Result<Html> {
    let user = require_auth(&req)?;
    require_capability(&user, Role::can_manage_mods)?;
    let q = query.q.as_deref().unwrap_or("").trim().to_string();

    if let Some(mod_id) = parse_forge_url(&q) {
        match state.forge.get_mod(mod_id, false).await {
            Ok(m) => {
                let tmpl = InstallSearchResultsTemplate {
                    results: vec![InstallSearchResult {
                        id: m.id,
                        name: m.name,
                        description: m.description,
                        fika_compatible: fika_compat_to_string(&m.fika_compatibility),
                    }],
                    error: None,
                };
                return Ok(Html::new(tmpl.render().map_err(WebError::from)?));
            }
            Err(_) => {
                let tmpl = InstallSearchResultsTemplate {
                    results: vec![],
                    error: Some(format!("Mod with ID {mod_id} not found on Forge.")),
                };
                return Ok(Html::new(tmpl.render().map_err(WebError::from)?));
            }
        }
    }

    if q.len() < 2 {
        let tmpl = InstallSearchResultsTemplate {
            results: vec![],
            error: None,
        };
        return Ok(Html::new(tmpl.render().map_err(WebError::from)?));
    }

    match state.forge.search_mods(&q).await {
        Ok(mods) => {
            let results = mods
                .into_iter()
                .map(|m| InstallSearchResult {
                    id: m.id,
                    name: m.name,
                    description: m.description,
                    fika_compatible: fika_compat_to_string(&m.fika_compatibility),
                })
                .collect();
            let tmpl = InstallSearchResultsTemplate {
                results,
                error: None,
            };
            Ok(Html::new(tmpl.render().map_err(WebError::from)?))
        }
        Err(_) => {
            let tmpl = InstallSearchResultsTemplate {
                results: vec![],
                error: Some("Could not reach SPT Forge. Try again later.".to_string()),
            };
            Ok(Html::new(tmpl.render().map_err(WebError::from)?))
        }
    }
}

pub async fn compat_check(
    state: Data<AppState>,
    req: HttpRequest,
    path: Path<i64>,
) -> actix_web::Result<Html> {
    let user = require_auth(&req)?;
    require_capability(&user, Role::can_manage_mods)?;
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

pub async fn install_mod(
    form: Form<InstallForm>,
    state: Data<AppState>,
    req: HttpRequest,
    session: Session,
) -> actix_web::Result<HttpResponse> {
    let user = require_auth(&req)?;
    require_capability(&user, Role::can_manage_mods)?;
    if !crate::web::csrf::validate_token(&session, &form.csrf_token) {
        return Err(WebError::Forbidden.into());
    }
    let mod_ref = form.mod_ref.trim();

    if mod_ref.is_empty() {
        return Err(WebError::BadRequest("Mod reference is required".to_string()).into());
    }

    let mod_id: i64 = match mod_ref.parse() {
        Ok(id) => id,
        Err(_) => match state.forge.search_mods(mod_ref).await {
            Ok(results) if results.len() == 1 => results[0].id,
            Ok(results) if results.is_empty() => {
                return Err(
                    WebError::BadRequest(format!("No mods found matching '{mod_ref}'")).into(),
                );
            }
            Ok(_) => {
                return Err(WebError::BadRequest(format!(
                    "Multiple mods match '{mod_ref}' — use a Forge mod ID instead"
                ))
                .into());
            }
            Err(_) => {
                return Err(WebError::BadRequest(
                    "Failed to search mods. Please try again.".to_string(),
                )
                .into());
            }
        },
    };

    let versions = state
        .forge
        .get_versions(mod_id, Some(&state.spt_info.spt_version))
        .await
        .map_err(WebError::from)?;

    let version = versions.last().ok_or(WebError::BadRequest(
        "No compatible version found for current SPT version".to_string(),
    ))?;

    let mod_info = state
        .forge
        .get_mod(mod_id, false)
        .await
        .map_err(WebError::from)?;

    const FIKA_FORGE_MOD_ID: i64 = 2326;

    // Check Fika compatibility if Fika is installed
    {
        let db = state.db.clone();
        let fika_installed = web::block(move || {
            let db = db.lock();
            Ok::<_, anyhow::Error>(db.get_mod_by_forge_id(FIKA_FORGE_MOD_ID)?.is_some())
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
                        "warning",
                    );
                }
                Some(FikaCompat::Unknown) => {
                    set_flash(
                        &session,
                        &format!(
                            "Note: Fika compatibility for {} v{} is unknown.",
                            mod_info.name, version.version
                        ),
                        "warning",
                    );
                }
                _ => {}
            }
        }
    }

    // Check if the operation should be queued (server running + queue enabled)
    let should_queue = crate::queue::should_queue(
        &state.config,
        false,
        &state.spt_dir,
        state.container_mgr.as_deref(),
    )
    .await
    .unwrap_or(false);

    if should_queue {
        let db = state.db.clone();
        let mod_name = mod_info.name.clone();
        let version_id = version.id;
        web::block(move || {
            let db = db.lock();
            db.insert_pending_op("install", mod_id, Some(version_id), &mod_name, None, None)
        })
        .await
        .map_err(WebError::from)?
        .map_err(WebError::from)?;

        set_flash(&session, "Mod queued for install", "success");
        return Ok(HttpResponse::SeeOther()
            .insert_header(("Location", "/quma/queue"))
            .finish());
    }

    // Prevent duplicate installs
    if state.tasks.has_running_for_mod(mod_id) {
        set_flash(&session, "This mod is already being installed", "warning");
        return Ok(HttpResponse::SeeOther()
            .insert_header(("Location", "/quma/mods"))
            .finish());
    }

    let task_id = state.tasks.start("Installing", &mod_info.name, mod_id);
    let tasks = state.tasks.clone();
    let forge = state.forge.clone();
    let spt_dir = state.spt_dir.clone();
    let db = state.db.clone();
    let config = state.config.clone();
    let version = version.clone();
    let mod_name = mod_info.name.clone();
    let mod_slug = mod_info.slug.clone();
    let update_cache = state.update_cache.clone();
    let state_clone = state.clone();

    tokio::spawn(async move {
        let result = async {
            let link = version
                .link
                .as_deref()
                .ok_or_else(|| anyhow::anyhow!("version has no download link"))?;
            let tmp_dir = tempfile::tempdir()?;
            let archive_path = tmp_dir.path().join("mod.zip");
            forge.download_file(link, &archive_path).await?;

            // Extract outside the DB lock — this is the slow part (file I/O)
            let spt_dir2 = spt_dir.clone();
            let extracted = actix_web::web::block(move || {
                crate::spt::mods::extract_mod(&archive_path, &spt_dir2)
            })
            .await??;

            // Only hold the lock for DB writes
            let version_id = version.id;
            let version_str = version.version.clone();
            let spt_dir2 = spt_dir.clone();
            let spt_dir3 = spt_dir.clone();
            let db2 = db.clone();
            let db3 = db.clone();
            let config2 = config.clone();
            let db_id = actix_web::web::block(move || {
                let db = db.lock();
                let db_id = db.insert_mod(
                    mod_id,
                    version_id,
                    &mod_name,
                    mod_slug.as_deref(),
                    &version_str,
                )?;
                for file in &extracted {
                    db.insert_file(db_id, &file.path, Some(&file.hash), Some(file.size as i64))?;
                }
                Ok::<_, anyhow::Error>(db_id)
            })
            .await??;

            // Scan for runtime-generated files and track them separately
            let _ = actix_web::web::block(move || {
                crate::ops::scan_and_record_runtime_files(&db2, db_id, &spt_dir2)
            })
            .await;

            // Regenerate NarcoNet config if enabled
            let _ = actix_web::web::block(move || {
                let db = db3.lock();
                crate::modsync::regenerate_if_enabled(&spt_dir3, &config2, &db)
            })
            .await;

            Ok::<_, anyhow::Error>(())
        }
        .await;

        match result {
            Ok(()) => {
                tracing::info!(mod_id, "mod installed successfully");
                update_cache.invalidate();
                // Re-check NarcoNet detection (installing NarcoNet itself changes this)
                state_clone.modsync_installed.store(
                    crate::config::is_modsync_installed(&spt_dir),
                    std::sync::atomic::Ordering::Relaxed,
                );
                if mod_id == 236 {
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
                tracing::error!(mod_id, error = %e, "mod install failed");
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
    require_capability(&user, Role::can_manage_mods)?;
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

    let versions = state
        .forge
        .get_versions(installed.forge_mod_id, Some(&state.spt_info.spt_version))
        .await
        .map_err(WebError::from)?;

    let version = versions.last().ok_or(WebError::BadRequest(
        "No compatible update found".to_string(),
    ))?;

    if version.version == installed.version {
        set_flash(&session, "Already up to date", "warning");
        return Ok(HttpResponse::SeeOther()
            .insert_header(("Location", format!("/quma/mods/{mod_db_id}")))
            .finish());
    }

    // Check if the operation should be queued
    let should_queue = crate::queue::should_queue(
        &state.config,
        false,
        &state.spt_dir,
        state.container_mgr.as_deref(),
    )
    .await
    .unwrap_or(false);

    if should_queue {
        let db = state.db.clone();
        let mod_name = installed.name.clone();
        let version_id = version.id;
        let forge_mod_id = installed.forge_mod_id;
        web::block(move || {
            let db = db.lock();
            db.insert_pending_op(
                "update",
                forge_mod_id,
                Some(version_id),
                &mod_name,
                None,
                None,
            )
        })
        .await
        .map_err(WebError::from)?
        .map_err(WebError::from)?;

        set_flash(&session, "Update queued", "success");
        return Ok(HttpResponse::SeeOther()
            .insert_header(("Location", "/quma/queue"))
            .finish());
    }

    // Prevent duplicate updates
    if state.tasks.has_running_for_mod(installed.forge_mod_id) {
        set_flash(&session, "This mod is already being updated", "warning");
        return Ok(HttpResponse::SeeOther()
            .insert_header(("Location", format!("/quma/mods/{mod_db_id}")))
            .finish());
    }

    let task_id = state
        .tasks
        .start("Updating", &installed.name, installed.forge_mod_id);
    let tasks = state.tasks.clone();
    let forge = state.forge.clone();
    let spt_dir = state.spt_dir.clone();
    let db = state.db.clone();
    let config = state.config.clone();
    let version = version.clone();
    let update_cache = state.update_cache.clone();

    tokio::spawn(async move {
        let result = async {
            let link = version
                .link
                .as_deref()
                .ok_or_else(|| anyhow::anyhow!("version has no download link"))?;
            let tmp_dir = tempfile::tempdir()?;
            let archive_path = tmp_dir.path().join("mod.zip");
            forge.download_file(link, &archive_path).await?;

            // Extract to staging outside the DB lock — this is the slow part
            let staging_dir = tempfile::tempdir()?;
            let staging_path = staging_dir.path().to_path_buf();
            let archive = archive_path.clone();
            let extracted = actix_web::web::block(move || {
                crate::spt::mods::extract_mod(&archive, &staging_path)
            })
            .await??;

            // Hold the lock only for file swap + DB writes
            let version_id = version.id;
            let version_str = version.version.clone();
            let staging_path = staging_dir.path().to_path_buf();
            let spt_dir2 = spt_dir.clone();
            let db2 = db.clone();
            let config2 = config.clone();
            actix_web::web::block(move || {
                let db = db.lock();
                let old_files = db.get_files_for_mod(mod_db_id)?;
                let old_paths: Vec<String> = old_files.into_iter().map(|f| f.file_path).collect();
                crate::spt::mods::delete_mod_files(&spt_dir, &old_paths)?;
                db.delete_files_for_mod(mod_db_id)?;

                for file in &extracted {
                    let src = staging_path.join(&file.path);
                    let dst = spt_dir.join(&file.path);
                    if let Some(parent) = dst.parent() {
                        std::fs::create_dir_all(parent)?;
                    }
                    std::fs::rename(&src, &dst)
                        .or_else(|_| std::fs::copy(&src, &dst).map(|_| ()))?;
                }

                for file in &extracted {
                    db.insert_file(
                        mod_db_id,
                        &file.path,
                        Some(&file.hash),
                        Some(file.size as i64),
                    )?;
                }
                db.update_mod(mod_db_id, version_id, &version_str)?;
                Ok::<_, anyhow::Error>(())
            })
            .await??;

            // Regenerate NarcoNet config if enabled
            let _ = actix_web::web::block(move || {
                let db = db2.lock();
                crate::modsync::regenerate_if_enabled(&spt_dir2, &config2, &db)
            })
            .await;

            Ok::<_, anyhow::Error>(())
        }
        .await;

        match result {
            Ok(()) => {
                tracing::info!(mod_db_id, "mod updated successfully");
                update_cache.invalidate();
                tasks.complete(task_id, "Mod updated successfully".to_string());
            }
            Err(e) => {
                tracing::error!(mod_db_id, error = %e, "mod update failed");
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
    require_capability(&user, Role::can_manage_mods)?;
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

    // Check if the operation should be queued
    let should_queue = crate::queue::should_queue(
        &state.config,
        false,
        &state.spt_dir,
        state.container_mgr.as_deref(),
    )
    .await
    .unwrap_or(false);

    if should_queue {
        let db = state.db.clone();
        let mod_name = installed.name.clone();
        let forge_mod_id = installed.forge_mod_id;
        web::block(move || {
            let db = db.lock();
            db.insert_pending_op("remove", forge_mod_id, None, &mod_name, None, None)
        })
        .await
        .map_err(WebError::from)?
        .map_err(WebError::from)?;

        set_flash(&session, "Mod queued for removal", "success");
        return Ok(HttpResponse::SeeOther()
            .insert_header(("Location", "/quma/queue"))
            .finish());
    }

    let spt_dir = state.spt_dir.clone();
    let db = state.db.clone();
    let config = state.config.clone();

    tracing::info!(mod_db_id, mod_name = %installed.name, "removing mod");
    web::block(move || {
        let db = db.lock();
        crate::ops::remove_mod_by_id(&db, &spt_dir, &config, mod_db_id)
    })
    .await
    .map_err(WebError::from)?
    .map_err(WebError::from)?;

    state.update_cache.invalidate();
    // Re-check NarcoNet detection (removing NarcoNet itself changes this)
    state.modsync_installed.store(
        crate::config::is_modsync_installed(&state.spt_dir),
        std::sync::atomic::Ordering::Relaxed,
    );
    if installed.forge_mod_id == 236 {
        state
            .svm_installed
            .store(false, std::sync::atomic::Ordering::Relaxed);
        tracing::info!("SVM removed — config editor disabled");
    }
    set_flash(&session, "Mod removed", "success");
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
    require_capability(&user, Role::can_manage_mods)?;
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
    let was_disabled = installed.disabled;
    let mod_name = installed.name.clone();

    web::block(move || {
        let db = db.lock();
        if was_disabled {
            crate::ops::enable_mod(&db, &spt_dir, mod_db_id)
        } else {
            crate::ops::disable_mod(&db, &spt_dir, mod_db_id)
        }
    })
    .await
    .map_err(WebError::from)?
    .map_err(WebError::from)?;

    if was_disabled {
        set_flash(&session, &format!("{mod_name} has been enabled"), "success");
    } else {
        set_flash(
            &session,
            &format!("{mod_name} has been disabled"),
            "success",
        );
    }
    Ok(HttpResponse::SeeOther()
        .insert_header(("Location", format!("/mods/{mod_db_id}")))
        .finish())
}

pub async fn update_all_mods(
    state: Data<AppState>,
    req: HttpRequest,
    session: Session,
    form: Form<crate::web::csrf::CsrfForm>,
) -> actix_web::Result<HttpResponse> {
    let user = require_auth(&req)?;
    require_capability(&user, Role::can_manage_mods)?;
    if !crate::web::csrf::validate_token(&session, &form.csrf_token) {
        return Err(WebError::Forbidden.into());
    }
    let db = state.db.clone();
    let installed = web::block(move || {
        let db = db.lock();
        db.list_mods()
    })
    .await
    .map_err(WebError::from)?
    .map_err(WebError::from)?;

    if installed.is_empty() {
        return Ok(HttpResponse::SeeOther()
            .insert_header(("Location", "/quma/mods"))
            .finish());
    }

    let check_list: Vec<(i64, String)> = installed
        .iter()
        .map(|m| (m.forge_mod_id, m.version.clone()))
        .collect();

    let results = state
        .forge
        .check_updates(&check_list, &state.spt_info.spt_version)
        .await
        .map_err(WebError::from)?;

    // Check if operations should be queued (server running + queue enabled)
    let should_queue = crate::queue::should_queue(
        &state.config,
        false,
        &state.spt_dir,
        state.container_mgr.as_deref(),
    )
    .await
    .unwrap_or(false);

    if should_queue {
        let db = state.db.clone();
        web::block(move || {
            let db = db.lock();
            for update in &results.updates {
                db.insert_pending_op(
                    "update",
                    update.current_version.mod_id,
                    Some(update.recommended_version.id),
                    &update.current_version.name,
                    None,
                    None,
                )?;
            }
            Ok::<_, anyhow::Error>(())
        })
        .await
        .map_err(WebError::from)?
        .map_err(WebError::from)?;

        set_flash(&session, "All updates queued", "success");
        return Ok(HttpResponse::SeeOther()
            .insert_header(("Location", "/quma/queue"))
            .finish());
    }

    if state.tasks.has_active() {
        set_flash(
            &session,
            "Please wait for current operations to finish before updating all",
            "warning",
        );
        return Ok(HttpResponse::SeeOther()
            .insert_header(("Location", "/quma/mods"))
            .finish());
    }

    let task_id = state.tasks.start("Updating", "all mods", 0);
    let tasks = state.tasks.clone();
    let forge = state.forge.clone();
    let spt_dir = state.spt_dir.clone();
    let db = state.db.clone();
    let config = state.config.clone();
    let installed = installed.clone();
    let update_cache = state.update_cache.clone();
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
                .find(|m| m.forge_mod_id == update.current_version.mod_id)
            {
                Some(m) => m,
                None => continue,
            };
            let mod_db_id = mod_db.id;

            let result = async {
                let tmp_dir = tempfile::tempdir()?;
                let archive_path = tmp_dir.path().join("mod.zip");
                forge.download_file(&link, &archive_path).await?;

                // Extract to staging outside the DB lock
                let staging_dir = tempfile::tempdir()?;
                let staging_path = staging_dir.path().to_path_buf();
                let archive = archive_path.clone();
                let extracted = actix_web::web::block(move || {
                    crate::spt::mods::extract_mod(&archive, &staging_path)
                })
                .await??;

                // Hold the lock only for file swap + DB writes
                let spt_dir = spt_dir.clone();
                let db = db.clone();
                let version_id = update.recommended_version.id;
                let version_str = update.recommended_version.version.clone();
                let staging_path = staging_dir.path().to_path_buf();

                actix_web::web::block(move || {
                    let db = db.lock();
                    let old_files = db.get_files_for_mod(mod_db_id)?;
                    let old_paths: Vec<String> =
                        old_files.into_iter().map(|f| f.file_path).collect();
                    crate::spt::mods::delete_mod_files(&spt_dir, &old_paths)?;
                    db.delete_files_for_mod(mod_db_id)?;

                    for file in &extracted {
                        let src = staging_path.join(&file.path);
                        let dst = spt_dir.join(&file.path);
                        if let Some(parent) = dst.parent() {
                            std::fs::create_dir_all(parent)?;
                        }
                        std::fs::rename(&src, &dst)
                            .or_else(|_| std::fs::copy(&src, &dst).map(|_| ()))?;
                    }

                    for file in &extracted {
                        db.insert_file(
                            mod_db_id,
                            &file.path,
                            Some(&file.hash),
                            Some(file.size as i64),
                        )?;
                    }
                    db.update_mod(mod_db_id, version_id, &version_str)?;
                    Ok::<_, anyhow::Error>(())
                })
                .await??;
                Ok::<_, anyhow::Error>(())
            }
            .await;

            match result {
                Ok(()) => success_count += 1,
                Err(e) => tracing::error!(mod_db_id, error = %e, "update failed during update-all"),
            }
        }

        // Regenerate NarcoNet config after all updates
        let spt_dir2 = spt_dir.clone();
        let db2 = db.clone();
        let config2 = config.clone();
        let _ = actix_web::web::block(move || {
            let db = db2.lock();
            crate::modsync::regenerate_if_enabled(&spt_dir2, &config2, &db)
        })
        .await;

        update_cache.invalidate();
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

    let filtered_entries = web::block(move || {
        let db = db.lock();
        let filtered = db.list_mods_filtered(&filter)?;
        Ok::<_, anyhow::Error>(filtered)
    })
    .await
    .map_err(WebError::from)?
    .map_err(WebError::from)?;

    let mods: Vec<ModListEntry> = filtered_entries
        .into_iter()
        .map(|(mod_info, file_count, total_size)| ModListEntry {
            mod_info,
            file_count,
            total_size,
        })
        .filter(|e| !is_infrastructure_mod(e.mod_info.forge_mod_id))
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
