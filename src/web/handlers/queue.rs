use actix_session::Session;
use actix_web::web::{self, Data, Html, Path};
use actix_web::HttpResponse;
use askama::Template;

use crate::db::users::PendingOperation;
use crate::web::auth::{require_admin, require_auth, SessionUser};
use crate::web::error::WebError;
use crate::web::state::AppState;

#[derive(Template)]
#[template(path = "queue.html")]
struct QueueTemplate {
    user: SessionUser,
    ops: Vec<PendingOperation>,
}

pub async fn queue_page(state: Data<AppState>, session: Session) -> actix_web::Result<Html> {
    let user = require_auth(&session)?;
    let db = state.db.clone();

    let ops = web::block(move || {
        let db = db.lock();
        db.list_pending_ops()
    })
    .await
    .map_err(WebError::from)?
    .map_err(WebError::from)?;

    let tmpl = QueueTemplate { user, ops };
    Ok(Html::new(tmpl.render().map_err(WebError::from)?))
}

pub async fn cancel_op(
    state: Data<AppState>,
    path: Path<i64>,
    session: Session,
) -> actix_web::Result<HttpResponse> {
    require_admin(&session)?;
    let op_id = path.into_inner();
    let db = state.db.clone();

    web::block(move || {
        let db = db.lock();
        db.delete_pending_op(op_id)
    })
    .await
    .map_err(WebError::from)?
    .map_err(WebError::from)?;

    Ok(HttpResponse::SeeOther()
        .insert_header(("Location", "/queue"))
        .finish())
}

pub async fn apply_queue(
    state: Data<AppState>,
    session: Session,
) -> actix_web::Result<HttpResponse> {
    require_admin(&session)?;
    let server_running = crate::server_detect::is_server_running(&state.config, &state.spt_dir)
        .await
        .unwrap_or(false);

    if server_running {
        return Ok(HttpResponse::BadRequest()
            .body("Cannot apply queue while SPT server is running. Stop the server first."));
    }

    let db = state.db.clone();
    let ops = web::block(move || {
        let db = db.lock();
        db.list_pending_ops()
    })
    .await
    .map_err(WebError::from)?
    .map_err(WebError::from)?;

    for op in &ops {
        let db = state.db.clone();
        let spt_dir = state.spt_dir.clone();
        let op_id = op.id;
        let forge_mod_id = op.forge_mod_id;

        match op.action.as_str() {
            "install" => {
                if let Some(version_id) = op.forge_version_id {
                    let forge_mod = state.forge.get_mod(op.forge_mod_id, true).await;
                    if let Ok(forge_mod) = forge_mod {
                        let versions = forge_mod.versions.unwrap_or_default();
                        if let Some(version) = versions.iter().find(|v| v.id == version_id) {
                            if let Some(link) = &version.link {
                                let tmp_dir = tempfile::tempdir().ok();
                                if let Some(tmp_dir) = tmp_dir {
                                    let archive_path = tmp_dir.path().join("mod.zip");
                                    if state.forge.download_file(link, &archive_path).await.is_ok()
                                    {
                                        let mod_name = op.mod_name.clone();
                                        let version_str = version.version.clone();
                                        let forge_mod_id = op.forge_mod_id;
                                        let _ = web::block(move || {
                                            use crate::spt::mods::extract_mod;
                                            let extracted = extract_mod(&archive_path, &spt_dir)?;
                                            let db = db.lock();
                                            let installed_id = db.insert_mod(
                                                forge_mod_id,
                                                version_id,
                                                &mod_name,
                                                None,
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
                                        .await;
                                    }
                                }
                            }
                        }
                    }
                }
            }
            "update" => {
                if let Some(version_id) = op.forge_version_id {
                    let forge_mod = state.forge.get_mod(forge_mod_id, true).await;
                    if let Ok(forge_mod) = forge_mod {
                        let versions = forge_mod.versions.unwrap_or_default();
                        if let Some(version) = versions.iter().find(|v| v.id == version_id) {
                            if let Some(link) = &version.link {
                                let tmp_dir = tempfile::tempdir().ok();
                                if let Some(tmp_dir) = tmp_dir {
                                    let archive_path = tmp_dir.path().join("mod.zip");
                                    if state.forge.download_file(link, &archive_path).await.is_ok()
                                    {
                                        let version_str = version.version.clone();
                                        let _ = web::block(move || {
                                            use crate::spt::mods::{delete_mod_files, extract_mod};
                                            let db = db.lock();
                                            if let Ok(Some(installed)) =
                                                db.get_mod_by_forge_id(forge_mod_id)
                                            {
                                                let old_files =
                                                    db.get_files_for_mod(installed.id)?;
                                                let old_paths: Vec<String> = old_files
                                                    .iter()
                                                    .map(|f| f.file_path.clone())
                                                    .collect();
                                                delete_mod_files(&spt_dir, &old_paths)?;
                                                db.delete_files_for_mod(installed.id)?;

                                                let extracted =
                                                    extract_mod(&archive_path, &spt_dir)?;
                                                for file in &extracted {
                                                    db.insert_file(
                                                        installed.id,
                                                        &file.path,
                                                        Some(&file.hash),
                                                        Some(file.size as i64),
                                                    )?;
                                                }
                                                db.update_mod(
                                                    installed.id,
                                                    version_id,
                                                    &version_str,
                                                )?;
                                            }
                                            Ok::<_, anyhow::Error>(())
                                        })
                                        .await;
                                    }
                                }
                            }
                        }
                    }
                }
            }
            "remove" => {
                let _ = web::block(move || {
                    use crate::spt::mods::delete_mod_files;
                    let db = db.lock();
                    if let Ok(Some(installed)) = db.get_mod_by_forge_id(forge_mod_id) {
                        let files = db.get_files_for_mod(installed.id)?;
                        let paths: Vec<String> =
                            files.iter().map(|f| f.file_path.clone()).collect();
                        delete_mod_files(&spt_dir, &paths)?;
                        db.delete_mod(installed.id)?;
                    }
                    Ok::<_, anyhow::Error>(())
                })
                .await;
            }
            _ => {}
        }

        let db = state.db.clone();
        let _ = web::block(move || {
            let db = db.lock();
            db.delete_pending_op(op_id)
        })
        .await;
    }

    Ok(HttpResponse::SeeOther()
        .insert_header(("Location", "/queue"))
        .finish())
}
