use actix_session::Session;
use actix_web::web::{self, Data, Form, Html, Path, Query};
use actix_web::HttpResponse;
use askama::Template;

use crate::db::mods::{InstalledFile, InstalledMod, ModDependency};
use crate::forge::models::DependencyNode;
use crate::web::auth::{get_session_user, SessionUser};
use crate::web::error::WebError;
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
}

#[derive(Template)]
#[template(path = "mods/detail.html")]
struct ModDetailTemplate {
    user: SessionUser,
    mod_info: InstalledMod,
    files: Vec<InstalledFile>,
    dependencies: Vec<DepEntry>,
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
}

#[derive(serde::Deserialize)]
pub struct DepTreeQuery {
    #[serde(rename = "mod")]
    mod_id: Option<i64>,
    ver: Option<String>,
}

// -- Handlers --

pub async fn list_mods(state: Data<AppState>, session: Session) -> actix_web::Result<Html> {
    let user = get_session_user(&session).unwrap();
    let db = state.db.clone();

    let mods = web::block(move || {
        let db = db.lock();
        let mods = db.list_mods()?;
        let mut entries = Vec::new();
        for m in mods {
            let file_count = db.get_files_for_mod(m.id)?.len();
            entries.push(ModListEntry {
                mod_info: m,
                file_count,
            });
        }
        Ok::<_, anyhow::Error>(entries)
    })
    .await
    .map_err(WebError::from)?
    .map_err(WebError::from)?;

    let tmpl = ModListTemplate { user, mods };
    Ok(Html::new(tmpl.render().map_err(WebError::from)?))
}

pub async fn mod_detail(
    state: Data<AppState>,
    session: Session,
    path: Path<i64>,
) -> actix_web::Result<Html> {
    let user = get_session_user(&session).unwrap();
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
    };
    Ok(Html::new(tmpl.render().map_err(WebError::from)?))
}

pub async fn check_updates_partial(state: Data<AppState>) -> actix_web::Result<Html> {
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
) -> actix_web::Result<Html> {
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
) -> actix_web::Result<HttpResponse> {
    let mod_ref = &form.mod_ref;

    let mod_id: i64 = match mod_ref.parse() {
        Ok(id) => id,
        Err(_) => match state.forge.search_mods(mod_ref).await {
            Ok(results) if results.len() == 1 => results[0].id,
            Ok(results) if results.is_empty() => {
                return Ok(
                    HttpResponse::BadRequest().body(format!("No mods found matching '{mod_ref}'"))
                );
            }
            Ok(_) => {
                return Ok(HttpResponse::BadRequest().body(format!(
                    "Multiple mods match '{mod_ref}' — use a Forge mod ID instead"
                )));
            }
            Err(e) => {
                return Ok(
                    HttpResponse::InternalServerError().body(format!("Forge API error: {e}"))
                );
            }
        },
    };

    let versions = state
        .forge
        .get_versions(mod_id, Some(&state.spt_info.spt_version))
        .await
        .map_err(|e| WebError::Internal(e))?;

    let version = versions.first().ok_or(WebError::BadRequest(
        "No compatible version found for current SPT version".to_string(),
    ))?;

    let mod_info = state
        .forge
        .get_mod(mod_id, false)
        .await
        .map_err(|e| WebError::Internal(e))?;

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

        return Ok(HttpResponse::SeeOther()
            .insert_header(("Location", "/queue"))
            .finish());
    }

    let link = version.link.as_deref().ok_or(WebError::BadRequest(
        "Version has no download link".to_string(),
    ))?;

    let tmp_dir = tempfile::tempdir().map_err(|e| WebError::Internal(e.into()))?;
    let archive_path = tmp_dir.path().join("mod.zip");
    state
        .forge
        .download_file(link, &archive_path)
        .await
        .map_err(|e| WebError::Internal(e))?;

    let spt_dir = state.spt_dir.clone();
    let db = state.db.clone();
    let version_id = version.id;
    let version_str = version.version.clone();
    let mod_name = mod_info.name.clone();
    let mod_slug = mod_info.slug.clone();

    web::block(move || {
        use crate::spt::mods::extract_mod;

        let extracted = extract_mod(&archive_path, &spt_dir)?;
        let db = db.lock();

        let installed_id = db.insert_mod(
            mod_id,
            version_id,
            &mod_name,
            mod_slug.as_deref(),
            &version_str,
        )?;

        for file in &extracted {
            db.insert_file(
                installed_id,
                &file.path,
                Some(&file.hash),
                Some(file.size as i64),
            )?;
        }

        Ok::<_, anyhow::Error>(())
    })
    .await
    .map_err(WebError::from)?
    .map_err(WebError::from)?;

    Ok(HttpResponse::SeeOther()
        .insert_header(("Location", "/mods"))
        .finish())
}

pub async fn update_mod(state: Data<AppState>, path: Path<i64>) -> actix_web::Result<HttpResponse> {
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
        .map_err(|e| WebError::Internal(e))?;

    let version = versions.first().ok_or(WebError::BadRequest(
        "No compatible update found".to_string(),
    ))?;

    if version.version == installed.version {
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

        return Ok(HttpResponse::SeeOther()
            .insert_header(("Location", "/queue"))
            .finish());
    }

    let link = version.link.as_deref().ok_or(WebError::BadRequest(
        "Version has no download link".to_string(),
    ))?;

    let tmp_dir = tempfile::tempdir().map_err(|e| WebError::Internal(e.into()))?;
    let archive_path = tmp_dir.path().join("mod.zip");
    state
        .forge
        .download_file(link, &archive_path)
        .await
        .map_err(|e| WebError::Internal(e))?;

    let spt_dir = state.spt_dir.clone();
    let db = state.db.clone();
    let version_id = version.id;
    let version_str = version.version.clone();

    web::block(move || {
        use crate::spt::mods::{delete_mod_files, extract_mod};

        let db = db.lock();

        let old_files = db.get_files_for_mod(mod_db_id)?;
        let old_paths: Vec<String> = old_files.iter().map(|f| f.file_path.clone()).collect();
        delete_mod_files(&spt_dir, &old_paths)?;
        db.delete_files_for_mod(mod_db_id)?;

        let extracted = extract_mod(&archive_path, &spt_dir)?;
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
    .await
    .map_err(WebError::from)?
    .map_err(WebError::from)?;

    Ok(HttpResponse::SeeOther()
        .insert_header(("Location", format!("/mods/{mod_db_id}")))
        .finish())
}

pub async fn remove_mod(state: Data<AppState>, path: Path<i64>) -> actix_web::Result<HttpResponse> {
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

        return Ok(HttpResponse::SeeOther()
            .insert_header(("Location", "/queue"))
            .finish());
    }

    let spt_dir = state.spt_dir.clone();
    let db = state.db.clone();

    web::block(move || {
        use crate::spt::mods::delete_mod_files;

        let db = db.lock();
        let files = db.get_files_for_mod(mod_db_id)?;
        let paths: Vec<String> = files.iter().map(|f| f.file_path.clone()).collect();
        delete_mod_files(&spt_dir, &paths)?;
        db.delete_mod(mod_db_id)?;
        Ok::<_, anyhow::Error>(())
    })
    .await
    .map_err(WebError::from)?
    .map_err(WebError::from)?;

    Ok(HttpResponse::SeeOther()
        .insert_header(("Location", "/mods"))
        .finish())
}

pub async fn update_all_mods(state: Data<AppState>) -> actix_web::Result<HttpResponse> {
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
        .map_err(|e| WebError::Internal(e))?;

    for update in &results.updates {
        let link = match &update.recommended_version.link {
            Some(l) => l.clone(),
            None => continue,
        };

        let mod_db = installed
            .iter()
            .find(|m| m.forge_mod_id == update.current_version.mod_id);
        let mod_db = match mod_db {
            Some(m) => m,
            None => continue,
        };
        let mod_db_id = mod_db.id;

        let tmp_dir = tempfile::tempdir().map_err(|e| WebError::Internal(e.into()))?;
        let archive_path = tmp_dir.path().join("mod.zip");
        state
            .forge
            .download_file(&link, &archive_path)
            .await
            .map_err(|e| WebError::Internal(e))?;

        let spt_dir = state.spt_dir.clone();
        let db = state.db.clone();
        let version_id = update.recommended_version.id;
        let version_str = update.recommended_version.version.clone();

        web::block(move || {
            use crate::spt::mods::{delete_mod_files, extract_mod};

            let db = db.lock();
            let old_files = db.get_files_for_mod(mod_db_id)?;
            let old_paths: Vec<String> = old_files.iter().map(|f| f.file_path.clone()).collect();
            delete_mod_files(&spt_dir, &old_paths)?;
            db.delete_files_for_mod(mod_db_id)?;

            let extracted = extract_mod(&archive_path, &spt_dir)?;
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
        .await
        .map_err(WebError::from)?
        .map_err(WebError::from)?;
    }

    Ok(HttpResponse::SeeOther()
        .insert_header(("Location", "/mods"))
        .finish())
}
