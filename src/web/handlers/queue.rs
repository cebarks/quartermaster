use std::sync::Arc;

use actix_session::Session;
use actix_web::web::{self, Data, Form, Html, Path};
use actix_web::{HttpRequest, HttpResponse};
use askama::Template;

use crate::db::rbac::Permission;
use crate::db::users::PendingOperation;
use crate::web::auth::{require_auth, require_permission, SessionUser};
use crate::web::error::WebError;
use crate::web::flash::{set_flash, FlashType};
use crate::web::state::AppState;

fn extract_request_id(metadata: Option<&str>) -> Option<i64> {
    metadata
        .and_then(|m| serde_json::from_str::<serde_json::Value>(m).ok())
        .and_then(|v| v.get("request_id")?.as_i64())
}

#[derive(Template)]
#[template(path = "partials/queue_content.html")]
struct QueueContentPartialTemplate {
    user: SessionUser,
    ops: Vec<PendingOperation>,
    csrf_token: String,
}

pub async fn queue_content_partial(
    state: Data<AppState>,
    req: HttpRequest,
    session: Session,
) -> actix_web::Result<Html> {
    let user = require_auth(&req)?;
    let csrf_token = crate::web::csrf::get_or_create_token(&session);
    let db = state.db.clone();
    let ops = web::block(move || {
        let db = db.lock();
        db.list_pending_ops()
    })
    .await
    .map_err(WebError::from)?
    .map_err(WebError::from)?;

    let tmpl = QueueContentPartialTemplate {
        user,
        ops,
        csrf_token,
    };
    Ok(Html::new(tmpl.render().map_err(WebError::from)?))
}

async fn cancel_op_inner(
    state: &Data<AppState>,
    session: &Session,
    user: &SessionUser,
    op_id: i64,
    request_status: Option<crate::db::requests::RequestStatus>,
    flash_msg: &str,
) -> actix_web::Result<HttpResponse> {
    let db = state.db.clone();

    let op = web::block(move || {
        let db = db.lock();
        db.list_pending_ops()
            .map(|ops| ops.into_iter().find(|o| o.id == op_id))
    })
    .await
    .map_err(WebError::from)?
    .map_err(WebError::from)?;

    // Save metadata before consuming op
    let metadata = op.as_ref().and_then(|o| o.metadata.clone());

    if let Some(ref op) = op {
        crate::queue::cleanup_queued_archive(op);
    }

    let db = state.db.clone();
    web::block(move || {
        let db = db.lock();
        db.delete_pending_op(op_id)
    })
    .await
    .map_err(WebError::from)?
    .map_err(WebError::from)?;

    // Transition linked request if applicable
    if let Some(new_status) = request_status {
        if let Some(request_id) = extract_request_id(metadata.as_deref()) {
            let db = state.db.clone();
            let user_id = user.user_id;
            let comment = flash_msg.to_string();
            let _ = web::block(move || {
                let db = db.lock();
                db.transition_request_status(
                    request_id,
                    &[crate::db::requests::RequestStatus::Queued],
                    new_status,
                    Some(user_id),
                    Some(&comment),
                )
            })
            .await;
        }
    }

    // Cascade cancel to dependency ops whose queued_for array references
    // this parent's forge_mod_id. Remove the parent ID from each dep's array;
    // cancel the dep entirely when the array becomes empty.
    let final_flash = if let Some(parent_forge_mod_id) = op.as_ref().and_then(|o| o.forge_mod_id) {
        let db = state.db.clone();
        let dep_result = web::block(move || {
            let db = db.lock();
            let deps = db.list_dep_ops_for_parent(parent_forge_mod_id)?;
            let mut cancelled = Vec::new();

            for dep in deps {
                let Some(ref metadata_str) = dep.metadata else {
                    continue;
                };
                let Ok(mut meta) = serde_json::from_str::<serde_json::Value>(metadata_str) else {
                    continue;
                };

                if let Some(arr) = meta.get_mut("queued_for").and_then(|v| v.as_array_mut()) {
                    arr.retain(|v| v.as_i64() != Some(parent_forge_mod_id));

                    if arr.is_empty() {
                        // No remaining parents — cancel this dep
                        crate::queue::cleanup_queued_archive(&dep);
                        db.delete_pending_op(dep.id)?;
                        cancelled.push(dep.mod_name.clone());
                    } else {
                        // Still needed by other parents — update metadata
                        let updated =
                            serde_json::to_string(&meta).unwrap_or_else(|_| metadata_str.clone());
                        db.update_pending_op_metadata(dep.id, &updated)?;
                    }
                }
            }
            Ok::<Vec<String>, anyhow::Error>(cancelled)
        })
        .await
        .map_err(WebError::from)?
        .map_err(WebError::from)?;

        if dep_result.is_empty() {
            flash_msg.to_string()
        } else {
            format!("{} (+ {} dep(s) cancelled)", flash_msg, dep_result.len())
        }
    } else {
        flash_msg.to_string()
    };

    set_flash(session, &final_flash, FlashType::Success);
    Ok(HttpResponse::SeeOther()
        .insert_header(("Location", "/quma/mods#queue"))
        .finish())
}

pub async fn cancel_op(
    state: Data<AppState>,
    path: Path<i64>,
    req: HttpRequest,
    session: Session,
    form: Form<crate::web::csrf::CsrfForm>,
) -> actix_web::Result<HttpResponse> {
    let user = require_auth(&req)?;
    require_permission(&user, Permission::QueueManage)?;
    if !crate::web::csrf::validate_token(&session, &form.csrf_token) {
        return Err(WebError::Forbidden.into());
    }
    cancel_op_inner(
        &state,
        &session,
        &user,
        path.into_inner(),
        Some(crate::db::requests::RequestStatus::Approved),
        "Operation cancelled",
    )
    .await
}

pub async fn cancel_and_reject_op(
    state: Data<AppState>,
    path: Path<i64>,
    req: HttpRequest,
    session: Session,
    form: Form<crate::web::csrf::CsrfForm>,
) -> actix_web::Result<HttpResponse> {
    let user = require_auth(&req)?;
    require_permission(&user, Permission::QueueManage)?;
    if !crate::web::csrf::validate_token(&session, &form.csrf_token) {
        return Err(WebError::Forbidden.into());
    }
    cancel_op_inner(
        &state,
        &session,
        &user,
        path.into_inner(),
        Some(crate::db::requests::RequestStatus::Rejected),
        "Operation cancelled and request rejected",
    )
    .await
}

pub async fn apply_queue(
    state: Data<AppState>,
    req: HttpRequest,
    session: Session,
    form: Form<crate::web::csrf::CsrfForm>,
) -> actix_web::Result<HttpResponse> {
    let user = require_auth(&req)?;
    require_permission(&user, Permission::QueueManage)?;
    if !crate::web::csrf::validate_token(&session, &form.csrf_token) {
        return Err(WebError::Forbidden.into());
    }
    let config = state.config_cloned();
    let server_running = crate::server_detect::is_server_running(
        &config,
        &state.dirs,
        state.container_mgr.as_deref(),
    )
    .await
    .unwrap_or(false);

    if server_running {
        set_flash(
            &session,
            "Cannot apply queue while server is running. Stop the server first.",
            FlashType::Error,
        );
        return Ok(HttpResponse::SeeOther()
            .insert_header(("Location", "/quma/mods#queue"))
            .finish());
    }

    let db = state.db.clone();
    let ops = web::block(move || {
        let db = db.lock();
        db.list_pending_ops()
    })
    .await
    .map_err(WebError::from)?
    .map_err(WebError::from)?;

    let task_id = match state.tasks.start_if_no_active(
        "Applying queue",
        &format!("{} operations", ops.len()),
        0,
    ) {
        Some(id) => id,
        None => {
            set_flash(
                &session,
                "Queue is already being applied. Please wait.",
                FlashType::Warning,
            );
            return Ok(HttpResponse::SeeOther()
                .insert_header(("Location", "/quma/mods#queue"))
                .finish());
        }
    };
    let tasks = state.tasks.clone();
    let state_clone = state.clone();

    tokio::spawn(async move {
        let mut failures: Vec<String> = Vec::new();
        let total = ops.len();

        for (i, op) in ops.iter().enumerate() {
            tasks.update_message(
                task_id,
                format!("Queue: {} {}/{} ({})", op.action, i + 1, total, op.mod_name),
            );

            let result = match op.action {
                crate::db::users::QueueAction::Install => apply_install(op, &state_clone).await,
                crate::db::users::QueueAction::Update => apply_update(op, &state_clone).await,
                crate::db::users::QueueAction::Remove => apply_remove(op, &state_clone).await,
            };

            match result {
                Ok(()) => {
                    crate::queue::cleanup_queued_archive(op);
                    let db = state_clone.db.clone();
                    let op_id = op.id;
                    if let Err(e) = web::block(move || {
                        let db = db.lock();
                        db.delete_pending_op(op_id)
                    })
                    .await
                    .map_err(|e| anyhow::anyhow!("{e}"))
                    .and_then(|r| r.map_err(|e| anyhow::anyhow!("{e}")))
                    {
                        tracing::error!(
                            mod_name = %op.mod_name,
                            err = %e,
                            "failed to delete pending op after successful apply"
                        );
                        failures.push(format!(
                            "{}: completed but queue entry not removed: {}",
                            op.mod_name, e
                        ));
                    }

                    // Transition linked request on install
                    if op.action == crate::db::users::QueueAction::Install {
                        if let Some(request_id) = extract_request_id(op.metadata.as_deref()) {
                            let db = state_clone.db.clone();
                            let _ = web::block(move || {
                                let db = db.lock();
                                db.transition_request_status(
                                    request_id,
                                    &[crate::db::requests::RequestStatus::Queued],
                                    crate::db::requests::RequestStatus::Installed,
                                    None,
                                    Some("Installed via queue"),
                                )
                            })
                            .await;
                        }
                    }

                    // Transition linked request on remove (installed → approved)
                    if op.action == crate::db::users::QueueAction::Remove {
                        if let Some(forge_mod_id) = op.forge_mod_id {
                            let db = state_clone.db.clone();
                            let _ = web::block(move || {
                                let db = db.lock();
                                db.transition_request_by_forge_mod_id(
                                    forge_mod_id,
                                    &[crate::db::requests::RequestStatus::Installed],
                                    crate::db::requests::RequestStatus::Approved,
                                    None,
                                    Some("Mod removed via queue"),
                                )
                            })
                            .await;
                        }
                    }
                }
                Err(e) => {
                    tracing::error!(
                        action = %op.action,
                        mod_name = %op.mod_name,
                        err = %format!("{e:#}"),
                        "queue apply failed"
                    );
                    failures.push(format!("{}: {e:#}", op.mod_name));
                }
            }
        }

        // Regenerate convoy catalog after all operations
        state_clone.regenerate_convoy();
        state_clone.mod_zip_cache.invalidate();
        state_clone.integrity_cache.invalidate();

        if failures.is_empty() {
            state_clone.clear_fika_items();
            tasks.complete(
                task_id,
                format!("Queue applied: {} operation(s) completed", total),
            );
        } else {
            state_clone.clear_fika_items();
            let msg = format!(
                "Queue: {} succeeded, {} failed — {}",
                total - failures.len(),
                failures.len(),
                failures.join("; "),
            );
            tasks.fail(task_id, msg);
        }
    });

    set_flash(&session, "Queue is being applied...", FlashType::Success);
    Ok(HttpResponse::SeeOther()
        .insert_header(("Location", "/quma/mods#queue"))
        .finish())
}

pub(super) async fn apply_install(op: &PendingOperation, state: &AppState) -> anyhow::Result<()> {
    if op.item_type == "addon" {
        return apply_addon_install(op, state).await;
    }

    let archive_path = op
        .archive_path
        .as_deref()
        .ok_or_else(|| anyhow::anyhow!("queued install for {} has no archive_path", op.mod_name))?;
    let archive = std::path::Path::new(archive_path);
    if !archive.exists() {
        anyhow::bail!("queued archive not found at {archive_path}");
    }

    let forge_mod_id = op.forge_mod_id;
    let version_id = op.forge_version_id;
    let source = crate::ops::ModSource::parse(&op.source).unwrap_or(crate::ops::ModSource::Forge);

    // Skip if already installed (dep may have been installed by a previous op in this batch)
    if let Some(fid) = forge_mod_id {
        let db = state.db.lock();
        if db.get_mod_by_forge_id(fid)?.is_some() {
            return Ok(());
        }
    }

    let version_str = crate::queue::extract_version_from_metadata(op.metadata.as_deref())
        .unwrap_or_else(|| "unknown".to_string());

    let queued_for = crate::queue::extract_queued_for(op.metadata.as_deref());

    let db = state.db.clone();
    let dirs = Arc::clone(&state.dirs);
    let config = state.config_cloned();
    let mod_name = op.mod_name.clone();
    let source_url = op.source_url.clone();
    let archive_owned = archive.to_path_buf();

    let installed_db_id = web::block(move || {
        let db = db.lock();
        crate::ops::install_mod_from_archive(&crate::ops::InstallRequest {
            db: &db,
            dirs: &dirs,
            config: &config,
            forge_mod_id,
            version_id,
            name: &mod_name,
            slug: None,
            version: &version_str,
            archive_path: &archive_owned,
            source,
            source_url: source_url.as_deref(),
        })
    })
    .await??;

    // Record dependency edges: if this op was queued as a dep (has queued_for),
    // each entry is a parent forge_mod_id that depends on us.
    if !queued_for.is_empty() {
        let db = state.db.lock();
        for parent_forge_mod_id in &queued_for {
            if let Ok(Some(parent)) = db.get_mod_by_forge_id(*parent_forge_mod_id) {
                match db.insert_dependency(
                    parent.id,
                    Some(installed_db_id),
                    op.forge_mod_id,
                    Some(&op.mod_name),
                    None,
                ) {
                    Ok(_) => {}
                    Err(rusqlite::Error::SqliteFailure(err, _))
                        if err.code == rusqlite::ffi::ErrorCode::ConstraintViolation => {}
                    Err(e) => {
                        tracing::warn!(
                            parent_id = parent.id,
                            dep_id = installed_db_id,
                            err = %e,
                            "failed to record dependency edge from queue"
                        );
                    }
                }
            }
        }
    }

    Ok(())
}

pub(super) async fn apply_update(op: &PendingOperation, state: &AppState) -> anyhow::Result<()> {
    if op.item_type == "addon" {
        return apply_addon_update(op, state).await;
    }

    let archive_path = op
        .archive_path
        .as_deref()
        .ok_or_else(|| anyhow::anyhow!("queued update for {} has no archive_path", op.mod_name))?;
    let archive = std::path::Path::new(archive_path);
    if !archive.exists() {
        anyhow::bail!("queued archive not found at {archive_path}");
    }

    let forge_mod_id = op
        .forge_mod_id
        .ok_or_else(|| anyhow::anyhow!("update op missing forge_mod_id"))?;
    let version_id = op
        .forge_version_id
        .ok_or_else(|| anyhow::anyhow!("update op missing version_id"))?;

    let version_str = crate::queue::extract_version_from_metadata(op.metadata.as_deref())
        .unwrap_or_else(|| "unknown".to_string());

    let db = state.db.clone();
    let dirs = Arc::clone(&state.dirs);
    let config = state.config_cloned();
    let archive_owned = archive.to_path_buf();

    web::block(move || {
        let db = db.lock();
        let installed = db
            .get_mod_by_forge_id(forge_mod_id)?
            .ok_or_else(|| anyhow::anyhow!("mod not found for forge_id {forge_mod_id}"))?;
        crate::ops::update_mod_from_archive(
            &db,
            &dirs,
            &config,
            installed.id,
            version_id,
            &version_str,
            &archive_owned,
        )
    })
    .await??;

    Ok(())
}

pub(super) async fn apply_remove(op: &PendingOperation, state: &AppState) -> anyhow::Result<()> {
    if op.item_type == "addon" {
        return apply_addon_remove(op, state).await;
    }

    let forge_mod_id = op
        .forge_mod_id
        .expect("mod operation must have forge_mod_id");
    let db = state.db.clone();
    let dirs = Arc::clone(&state.dirs);
    let config = state.config_cloned();

    web::block(move || {
        let db = db.lock();
        let installed = db
            .get_mod_by_forge_id(forge_mod_id)?
            .ok_or_else(|| anyhow::anyhow!("mod not found for forge_id {forge_mod_id}"))?;

        // Collect and remove reverse dependencies (same as CLI drain_all)
        let reverse_deps = crate::ops::collect_all_reverse_deps(&db, installed.id)?;
        for dep in reverse_deps.iter().rev() {
            crate::ops::remove_mod_by_id(&db, &dirs, &config, dep.id)?;
        }

        crate::ops::remove_mod_by_id(&db, &dirs, &config, installed.id)
    })
    .await??;

    Ok(())
}

// ── Addon Queue Operations ───────────────────────────────────────────

async fn apply_addon_install(op: &PendingOperation, state: &AppState) -> anyhow::Result<()> {
    let forge_addon_id = op
        .forge_addon_id
        .ok_or_else(|| anyhow::anyhow!("addon operation missing forge_addon_id"))?;
    let version_id = op
        .forge_version_id
        .ok_or_else(|| anyhow::anyhow!("addon install op missing version_id"))?;

    let archive_path = op.archive_path.as_deref().ok_or_else(|| {
        anyhow::anyhow!(
            "queued addon install for {} has no archive_path",
            op.mod_name
        )
    })?;
    let archive = std::path::Path::new(archive_path);
    if !archive.exists() {
        anyhow::bail!("queued archive not found at {archive_path}");
    }

    // Check if already installed
    {
        let db = state.db.lock();
        if db.get_addon_by_forge_id(forge_addon_id)?.is_some() {
            return Ok(());
        }
    }

    // Extract parent_forge_mod_id from metadata (stored at staging time)
    let metadata_val: serde_json::Value = op
        .metadata
        .as_deref()
        .and_then(|m| serde_json::from_str(m).ok())
        .unwrap_or(serde_json::Value::Null);
    let parent_forge_mod_id = metadata_val
        .get("parent_forge_mod_id")
        .and_then(|v| v.as_i64())
        .ok_or_else(|| anyhow::anyhow!("addon op missing parent_forge_mod_id in metadata"))?;
    let version_str = metadata_val
        .get("version")
        .and_then(|v| v.as_str())
        .unwrap_or("unknown")
        .to_string();
    let mod_version_constraint = metadata_val
        .get("mod_version_constraint")
        .and_then(|v| v.as_str())
        .map(String::from);

    // Find parent mod in DB
    let parent_mod_db_id = {
        let db = state.db.lock();
        let parent_mod = db
            .get_mod_by_forge_id(parent_forge_mod_id)?
            .ok_or_else(|| anyhow::anyhow!("parent mod {} not installed", parent_forge_mod_id))?;
        parent_mod.id
    };

    let db = state.db.clone();
    let dirs = Arc::clone(&state.dirs);
    let config = state.config_cloned();
    let addon_name = op.mod_name.clone();
    let archive_owned = archive.to_path_buf();

    web::block(move || {
        let db = db.lock();
        let req = crate::ops::InstallAddonRequest {
            db: &db,
            dirs: &dirs,
            config: &config,
            forge_addon_id: Some(forge_addon_id),
            parent_mod_id: parent_mod_db_id,
            version_id: Some(version_id),
            name: &addon_name,
            slug: None,
            version: &version_str,
            mod_version_constraint: mod_version_constraint.as_deref(),
            archive_path: &archive_owned,
            source: crate::ops::ModSource::Forge,
            source_url: None,
        };
        crate::ops::install_addon_from_archive(&req)?;
        Ok::<_, anyhow::Error>(())
    })
    .await??;

    Ok(())
}

async fn apply_addon_update(op: &PendingOperation, state: &AppState) -> anyhow::Result<()> {
    let forge_addon_id = op
        .forge_addon_id
        .ok_or_else(|| anyhow::anyhow!("addon operation missing forge_addon_id"))?;
    let version_id = op
        .forge_version_id
        .ok_or_else(|| anyhow::anyhow!("addon update op missing version_id"))?;

    let archive_path = op.archive_path.as_deref().ok_or_else(|| {
        anyhow::anyhow!(
            "queued addon update for {} has no archive_path",
            op.mod_name
        )
    })?;
    let archive = std::path::Path::new(archive_path);
    if !archive.exists() {
        anyhow::bail!("queued archive not found at {archive_path}");
    }

    // Get installed addon
    let addon_db_id = {
        let db = state.db.lock();
        let addon = db
            .get_addon_by_forge_id(forge_addon_id)?
            .ok_or_else(|| anyhow::anyhow!("addon not installed"))?;
        addon.id
    };

    // Extract version info from metadata
    let version_str = crate::queue::extract_version_from_metadata(op.metadata.as_deref())
        .unwrap_or_else(|| "unknown".to_string());
    let mod_version_constraint = op
        .metadata
        .as_deref()
        .and_then(|m| serde_json::from_str::<serde_json::Value>(m).ok())
        .and_then(|v| v.get("mod_version_constraint")?.as_str().map(String::from));

    let db = state.db.clone();
    let dirs = Arc::clone(&state.dirs);
    let config = state.config_cloned();
    let archive_owned = archive.to_path_buf();

    web::block(move || {
        let db = db.lock();
        crate::ops::update_addon_from_archive(
            &db,
            &dirs,
            &config,
            addon_db_id,
            version_id,
            &version_str,
            mod_version_constraint.as_deref(),
            &archive_owned,
        )
    })
    .await??;

    Ok(())
}

async fn apply_addon_remove(op: &PendingOperation, state: &AppState) -> anyhow::Result<()> {
    let forge_addon_id = op
        .forge_addon_id
        .ok_or_else(|| anyhow::anyhow!("addon operation missing forge_addon_id"))?;

    let db = state.db.clone();
    let dirs = Arc::clone(&state.dirs);
    let config = state.config_cloned();

    web::block(move || {
        let db = db.lock();
        let addon = db
            .get_addon_by_forge_id(forge_addon_id)?
            .ok_or_else(|| anyhow::anyhow!("addon not found for forge_id {forge_addon_id}"))?;
        crate::ops::remove_addon_by_id(&db, &dirs, &config, addon.id)
    })
    .await??;

    Ok(())
}
