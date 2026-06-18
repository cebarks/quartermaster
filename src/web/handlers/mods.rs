use actix_session::Session;
use actix_web::web::{self, Data, Form, Html, Path, Query};
use actix_web::HttpResponse;
use askama::Template;

use crate::db::mods::{InstalledFile, InstalledMod, ModDependency};
use crate::forge::models::DependencyNode;
use crate::web::auth::{require_admin, require_auth, SessionUser};
use crate::web::error::WebError;
use crate::web::flash::{set_flash, take_flash, FlashMessage};
use crate::web::state::AppState;

// -- View models --

struct ModListEntry {
    mod_info: InstalledMod,
    file_count: usize,
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
    mods: Vec<ModListEntry>,
    flash: Option<FlashMessage>,
    csrf_token: String,
}

#[derive(Template)]
#[template(path = "mods/detail.html")]
struct ModDetailTemplate {
    user: SessionUser,
    mod_info: InstalledMod,
    files: Vec<InstalledFile>,
    dependencies: Vec<DepEntry>,
    flash: Option<FlashMessage>,
    csrf_token: String,
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

// -- Form structs --

#[derive(serde::Deserialize)]
pub struct InstallForm {
    mod_ref: String,
    csrf_token: String,
}

#[derive(serde::Deserialize)]
pub struct DepTreeQuery {
    #[serde(rename = "mod")]
    mod_id: Option<i64>,
    ver: Option<String>,
}

// -- Handlers --

pub async fn list_mods(state: Data<AppState>, session: Session) -> actix_web::Result<Html> {
    let user = require_admin(&session)?;
    let flash = take_flash(&session);
    let csrf_token = crate::web::csrf::get_or_create_token(&session);
    let db = state.db.clone();

    let mods = web::block(move || {
        let db = db.lock();
        let mods_with_counts = db.list_mods_with_file_counts()?;
        let entries = mods_with_counts
            .into_iter()
            .map(|(mod_info, file_count)| ModListEntry {
                mod_info,
                file_count,
            })
            .collect();
        Ok::<_, anyhow::Error>(entries)
    })
    .await
    .map_err(WebError::from)?
    .map_err(WebError::from)?;

    let tmpl = ModListTemplate {
        user,
        mods,
        flash,
        csrf_token,
    };
    Ok(Html::new(tmpl.render().map_err(WebError::from)?))
}

pub async fn mod_detail(
    state: Data<AppState>,
    session: Session,
    path: Path<i64>,
) -> actix_web::Result<Html> {
    let user = require_auth(&session)?;
    let flash = take_flash(&session);
    let csrf_token = crate::web::csrf::get_or_create_token(&session);
    let mod_id = path.into_inner();
    let db = state.db.clone();

    let (mod_info, files, dependencies) = web::block(move || {
        let db = db.lock();
        let mod_info = db
            .get_mod(mod_id)?
            .ok_or_else(|| anyhow::anyhow!("mod not found"))?;
        let files = db.get_files_for_mod(mod_id)?;
        let deps = db.get_dependencies(mod_id)?;
        let mut dep_entries = Vec::new();
        for dep in deps {
            let dep_mod = db.get_mod(dep.depends_on_mod_id)?;
            dep_entries.push(DepEntry { dep, dep_mod });
        }
        Ok::<_, anyhow::Error>((mod_info, files, dep_entries))
    })
    .await
    .map_err(WebError::from)?
    .map_err(WebError::from)?;

    let tmpl = ModDetailTemplate {
        user,
        mod_info,
        files,
        dependencies,
        flash,
        csrf_token,
    };
    Ok(Html::new(tmpl.render().map_err(WebError::from)?))
}

pub async fn check_updates_partial(
    state: Data<AppState>,
    session: Session,
) -> actix_web::Result<Html> {
    require_admin(&session)?;
    let db = state.db.clone();
    let installed = web::block(move || {
        let db = db.lock();
        db.list_mods()
    })
    .await
    .map_err(WebError::from)?
    .map_err(WebError::from)?;

    let updates_available = if !installed.is_empty() {
        let check_list: Vec<(i64, String)> = installed
            .iter()
            .map(|m| (m.forge_mod_id, m.version.clone()))
            .collect();
        match state
            .forge
            .check_updates(&check_list, &state.spt_info.spt_version)
            .await
        {
            Ok(result) => result.updates.len(),
            Err(_) => 0,
        }
    } else {
        0
    };

    let tmpl = UpdateBadgesTemplate { updates_available };
    Ok(Html::new(tmpl.render().map_err(WebError::from)?))
}

pub async fn dep_tree_partial(
    state: Data<AppState>,
    query: Query<DepTreeQuery>,
    session: Session,
) -> actix_web::Result<Html> {
    require_admin(&session)?;
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

pub async fn install_mod(
    form: Form<InstallForm>,
    state: Data<AppState>,
    session: Session,
) -> actix_web::Result<HttpResponse> {
    require_admin(&session)?;
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

    let version = versions.first().ok_or(WebError::BadRequest(
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
    let should_queue = crate::queue::should_queue(&state.config, false, &state.spt_dir)
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
            .insert_header(("Location", "/queue"))
            .finish());
    }

    // Prevent duplicate installs
    if state.tasks.has_running_for_mod(mod_id) {
        set_flash(&session, "This mod is already being installed", "warning");
        return Ok(HttpResponse::SeeOther()
            .insert_header(("Location", "/mods"))
            .finish());
    }

    let task_id = state.tasks.start("Install", &mod_info.name, mod_id);
    let tasks = state.tasks.clone();
    let forge = state.forge.clone();
    let spt_dir = state.spt_dir.clone();
    let db = state.db.clone();
    let version = version.clone();
    let mod_name = mod_info.name.clone();
    let mod_slug = mod_info.slug.clone();

    tokio::spawn(async move {
        let result = async {
            let link = version
                .link
                .as_deref()
                .ok_or_else(|| anyhow::anyhow!("version has no download link"))?;
            let tmp_dir = tempfile::tempdir()?;
            let archive_path = tmp_dir.path().join("mod.zip");
            forge.download_file(link, &archive_path).await?;

            let version_id = version.id;
            let version_str = version.version.clone();
            actix_web::web::block(move || {
                let db = db.lock();
                crate::ops::install_mod_from_archive(
                    &db,
                    &spt_dir,
                    mod_id,
                    version_id,
                    &mod_name,
                    mod_slug.as_deref(),
                    &version_str,
                    &archive_path,
                )
            })
            .await??;
            Ok::<_, anyhow::Error>(())
        }
        .await;

        match result {
            Ok(()) => {
                tracing::info!(mod_id, "mod installed successfully");
                tasks.complete(task_id, "Mod installed successfully".to_string());
            }
            Err(e) => {
                tracing::error!(mod_id, error = %e, "mod install failed");
                tasks.fail(task_id, format!("Install failed: {e}"));
            }
        }
    });

    Ok(HttpResponse::SeeOther()
        .insert_header(("Location", "/mods"))
        .finish())
}

pub async fn update_mod(
    state: Data<AppState>,
    path: Path<i64>,
    session: Session,
    form: Form<crate::web::csrf::CsrfForm>,
) -> actix_web::Result<HttpResponse> {
    require_admin(&session)?;
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

    let version = versions.first().ok_or(WebError::BadRequest(
        "No compatible update found".to_string(),
    ))?;

    if version.version == installed.version {
        set_flash(&session, "Already up to date", "warning");
        return Ok(HttpResponse::SeeOther()
            .insert_header(("Location", format!("/mods/{mod_db_id}")))
            .finish());
    }

    // Check if the operation should be queued
    let should_queue = crate::queue::should_queue(&state.config, false, &state.spt_dir)
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
            .insert_header(("Location", "/queue"))
            .finish());
    }

    // Prevent duplicate updates
    if state.tasks.has_running_for_mod(installed.forge_mod_id) {
        set_flash(&session, "This mod is already being updated", "warning");
        return Ok(HttpResponse::SeeOther()
            .insert_header(("Location", format!("/mods/{mod_db_id}")))
            .finish());
    }

    let task_id = state
        .tasks
        .start("Updat", &installed.name, installed.forge_mod_id);
    let tasks = state.tasks.clone();
    let forge = state.forge.clone();
    let spt_dir = state.spt_dir.clone();
    let db = state.db.clone();
    let version = version.clone();

    tokio::spawn(async move {
        let result = async {
            let link = version
                .link
                .as_deref()
                .ok_or_else(|| anyhow::anyhow!("version has no download link"))?;
            let tmp_dir = tempfile::tempdir()?;
            let archive_path = tmp_dir.path().join("mod.zip");
            forge.download_file(link, &archive_path).await?;

            let version_id = version.id;
            let version_str = version.version.clone();
            actix_web::web::block(move || {
                let db = db.lock();
                crate::ops::update_mod_from_archive(
                    &db,
                    &spt_dir,
                    mod_db_id,
                    version_id,
                    &version_str,
                    &archive_path,
                )
            })
            .await??;
            Ok::<_, anyhow::Error>(())
        }
        .await;

        match result {
            Ok(()) => {
                tracing::info!(mod_db_id, "mod updated successfully");
                tasks.complete(task_id, "Mod updated successfully".to_string());
            }
            Err(e) => {
                tracing::error!(mod_db_id, error = %e, "mod update failed");
                tasks.fail(task_id, format!("Update failed: {e}"));
            }
        }
    });

    Ok(HttpResponse::SeeOther()
        .insert_header(("Location", format!("/mods/{mod_db_id}")))
        .finish())
}

pub async fn remove_mod(
    state: Data<AppState>,
    path: Path<i64>,
    session: Session,
    form: Form<crate::web::csrf::CsrfForm>,
) -> actix_web::Result<HttpResponse> {
    require_admin(&session)?;
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
    let should_queue = crate::queue::should_queue(&state.config, false, &state.spt_dir)
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
            .insert_header(("Location", "/queue"))
            .finish());
    }

    let spt_dir = state.spt_dir.clone();
    let db = state.db.clone();

    web::block(move || {
        let db = db.lock();
        crate::ops::remove_mod_by_id(&db, &spt_dir, mod_db_id)
    })
    .await
    .map_err(WebError::from)?
    .map_err(WebError::from)?;

    set_flash(&session, "Mod removed", "success");
    Ok(HttpResponse::SeeOther()
        .insert_header(("Location", "/mods"))
        .finish())
}

pub async fn update_all_mods(
    state: Data<AppState>,
    session: Session,
    form: Form<crate::web::csrf::CsrfForm>,
) -> actix_web::Result<HttpResponse> {
    require_admin(&session)?;
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
            .insert_header(("Location", "/mods"))
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
    let should_queue = crate::queue::should_queue(&state.config, false, &state.spt_dir)
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
            .insert_header(("Location", "/queue"))
            .finish());
    }

    let task_id = state.tasks.start("Updat", "all mods", 0);
    let tasks = state.tasks.clone();
    let forge = state.forge.clone();
    let spt_dir = state.spt_dir.clone();
    let db = state.db.clone();
    let installed = installed.clone();

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

                let spt_dir = spt_dir.clone();
                let db = db.clone();
                let version_id = update.recommended_version.id;
                let version_str = update.recommended_version.version.clone();

                actix_web::web::block(move || {
                    let db = db.lock();
                    crate::ops::update_mod_from_archive(
                        &db,
                        &spt_dir,
                        mod_db_id,
                        version_id,
                        &version_str,
                        &archive_path,
                    )
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

        if success_count == total {
            tasks.complete(task_id, format!("All {total} mods updated successfully"));
        } else {
            tasks.fail(task_id, format!("{success_count}/{total} mods updated"));
        }
    });

    Ok(HttpResponse::SeeOther()
        .insert_header(("Location", "/mods"))
        .finish())
}
