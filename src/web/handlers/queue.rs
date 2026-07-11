use actix_session::Session;
use actix_web::web::{self, Data, Form, Html, Path};
use actix_web::{HttpRequest, HttpResponse};
use askama::Template;

use crate::db::rbac::Permission;
use crate::db::users::PendingOperation;
use crate::dirs::QumaDirs;
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

    set_flash(session, flash_msg, FlashType::Success);
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
    let dirs = QumaDirs::from_legacy(state.spt_dir.clone());
    let server_running =
        crate::server_detect::is_server_running(&config, &dirs, state.container_mgr.as_deref())
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

async fn resolve_version_link(
    state: &AppState,
    forge_mod_id: i64,
    version_id: i64,
) -> anyhow::Result<(String, String, crate::forge::models::ForgeVersion)> {
    let forge_mod = state.forge.get_mod(forge_mod_id, true).await?;
    let versions = forge_mod.versions.unwrap_or_default();
    let version = versions
        .iter()
        .find(|v| v.id == version_id)
        .ok_or_else(|| anyhow::anyhow!("version {version_id} not found"))?;
    let link = version
        .link
        .as_deref()
        .ok_or_else(|| anyhow::anyhow!("version has no download link"))?;
    Ok((link.to_string(), version.version.clone(), version.clone()))
}

async fn download_to_temp(state: &AppState, link: &str) -> anyhow::Result<tempfile::TempDir> {
    let tmp_dir = tempfile::tempdir()?;
    let archive_path = tmp_dir.path().join("mod.zip");
    state.forge.download_file(link, &archive_path).await?;
    Ok(tmp_dir)
}

pub(super) async fn apply_install(op: &PendingOperation, state: &AppState) -> anyhow::Result<()> {
    if op.item_type == "addon" {
        return apply_addon_install(op, state).await;
    }

    // URL/file install — archive already downloaded
    if let Some(ref archive_path) = op.archive_path {
        let archive = std::path::Path::new(archive_path);
        if !archive.exists() {
            anyhow::bail!("queued archive not found at {archive_path}");
        }
        let source =
            crate::ops::ModSource::parse(&op.source).unwrap_or(crate::ops::ModSource::Forge);

        let db = state.db.clone();
        let dirs = QumaDirs::from_legacy(state.spt_dir.clone());
        let config = state.config_cloned();
        let mod_name = op.mod_name.clone();
        let source_url = op.source_url.clone();
        let archive_owned = archive.to_path_buf();

        web::block(move || {
            let db = db.lock();
            crate::ops::install_mod_from_archive(&crate::ops::InstallRequest {
                db: &db,
                dirs: &dirs,
                config: &config,
                forge_mod_id: None,
                version_id: None,
                name: &mod_name,
                slug: None,
                version: "unknown",
                archive_path: &archive_owned,
                source,
                source_url: source_url.as_deref(),
            })
        })
        .await??;

        // Clean up cached archive
        let _ = std::fs::remove_file(archive);
        return Ok(());
    }

    // Existing Forge install path
    let forge_mod_id = op
        .forge_mod_id
        .expect("mod operation must have forge_mod_id");
    let version_id = op
        .forge_version_id
        .ok_or_else(|| anyhow::anyhow!("install op missing version_id"))?;

    {
        let db = state.db.lock();
        if db.get_mod_by_forge_id(forge_mod_id)?.is_some() {
            return Ok(());
        }
    }

    let (link, version_str, full_version) =
        resolve_version_link(state, forge_mod_id, version_id).await?;

    let dirs = QumaDirs::from_legacy(state.spt_dir.clone());
    let dep_db_ids = crate::ops::resolve_and_install_deps(
        &state.forge,
        &state.db,
        &dirs,
        &state.config_cloned(),
        forge_mod_id,
        &full_version,
    )
    .await?;

    let tmp_dir = download_to_temp(state, &link).await?;
    let archive_path = tmp_dir.path().join("mod.zip");

    let db = state.db.clone();
    let dirs = QumaDirs::from_legacy(state.spt_dir.clone());
    let config = state.config_cloned();
    let mod_name = op.mod_name.clone();

    let db_id = web::block(move || {
        let db = db.lock();
        if let Some(existing) = db.get_mod_by_forge_id(forge_mod_id)? {
            return Ok(Some(existing.id));
        }
        let id = crate::ops::install_mod_from_archive(&crate::ops::InstallRequest {
            db: &db,
            dirs: &dirs,
            config: &config,
            forge_mod_id: Some(forge_mod_id),
            version_id: Some(version_id),
            name: &mod_name,
            slug: None,
            version: &version_str,
            archive_path: &archive_path,
            source: crate::ops::ModSource::Forge,
            source_url: None,
        })?;
        Ok::<_, anyhow::Error>(Some(id))
    })
    .await??;

    if let Some(db_id) = db_id {
        crate::ops::record_dep_edges(&state.db, db_id, &dep_db_ids);
    }

    Ok(())
}

pub(super) async fn apply_update(op: &PendingOperation, state: &AppState) -> anyhow::Result<()> {
    if op.item_type == "addon" {
        return apply_addon_update(op, state).await;
    }

    let forge_mod_id = op
        .forge_mod_id
        .expect("mod operation must have forge_mod_id");
    let version_id = op
        .forge_version_id
        .ok_or_else(|| anyhow::anyhow!("update op missing version_id"))?;
    let (link, version_str, full_version) =
        resolve_version_link(state, forge_mod_id, version_id).await?;

    let dirs = QumaDirs::from_legacy(state.spt_dir.clone());
    let dep_db_ids = crate::ops::resolve_and_install_deps(
        &state.forge,
        &state.db,
        &dirs,
        &state.config_cloned(),
        forge_mod_id,
        &full_version,
    )
    .await?;

    let tmp_dir = download_to_temp(state, &link).await?;
    let archive_path = tmp_dir.path().join("mod.zip");

    let db = state.db.clone();
    let dirs = QumaDirs::from_legacy(state.spt_dir.clone());
    let config = state.config_cloned();

    let mod_db_id = web::block(move || {
        let db = db.lock();
        let installed = db
            .get_mod_by_forge_id(forge_mod_id)?
            .ok_or_else(|| anyhow::anyhow!("mod not found for forge_id {forge_mod_id}"))?;
        let db_id = installed.id;
        crate::ops::update_mod_from_archive(
            &db,
            &dirs,
            &config,
            db_id,
            version_id,
            &version_str,
            &archive_path,
        )?;
        Ok::<_, anyhow::Error>(db_id)
    })
    .await??;

    crate::ops::record_dep_edges(&state.db, mod_db_id, &dep_db_ids);

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
    let dirs = QumaDirs::from_legacy(state.spt_dir.clone());
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

    // Check if already installed
    {
        let db = state.db.lock();
        if db.get_addon_by_forge_id(forge_addon_id)?.is_some() {
            return Ok(());
        }
    }

    // Fetch addon info
    let addon_info = state.forge.get_addon(forge_addon_id, true).await?;
    let versions = addon_info.versions.unwrap_or_default();
    let version = versions
        .iter()
        .find(|v| v.id == version_id)
        .ok_or_else(|| anyhow::anyhow!("version {version_id} not found"))?;
    let link = version
        .link
        .as_deref()
        .ok_or_else(|| anyhow::anyhow!("version has no download link"))?;

    // Find parent mod by forge_mod_id
    let parent_forge_mod_id = addon_info
        .mod_id
        .ok_or_else(|| anyhow::anyhow!("addon has no parent mod_id"))?;
    let parent_mod_db_id = {
        let db = state.db.lock();
        let parent_mod = db
            .get_mod_by_forge_id(parent_forge_mod_id)?
            .ok_or_else(|| anyhow::anyhow!("parent mod {} not installed", parent_forge_mod_id))?;
        parent_mod.id
    };

    // Download
    let tmp_dir = tempfile::tempdir()?;
    let archive_path = tmp_dir.path().join("addon.zip");
    state.forge.download_file(link, &archive_path).await?;

    // Install
    let db = state.db.clone();
    let dirs = QumaDirs::from_legacy(state.spt_dir.clone());
    let config = state.config_cloned();
    let addon_name = op.mod_name.clone();
    let addon_slug = addon_info.slug.clone();
    let version_str = version.version.clone();
    let mod_version_constraint = version.mod_version_constraint.clone();

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
            slug: addon_slug.as_deref(),
            version: &version_str,
            mod_version_constraint: mod_version_constraint.as_deref(),
            archive_path: &archive_path,
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

    // Get installed addon
    let addon_db_id = {
        let db = state.db.lock();
        let addon = db
            .get_addon_by_forge_id(forge_addon_id)?
            .ok_or_else(|| anyhow::anyhow!("addon not installed"))?;
        addon.id
    };

    // Fetch version info
    let addon_info = state.forge.get_addon(forge_addon_id, true).await?;
    let versions = addon_info.versions.unwrap_or_default();
    let version = versions
        .iter()
        .find(|v| v.id == version_id)
        .ok_or_else(|| anyhow::anyhow!("version {version_id} not found"))?;
    let link = version
        .link
        .as_deref()
        .ok_or_else(|| anyhow::anyhow!("version has no download link"))?;

    // Download archive to system temp, extract to same-fs staging dir
    let tmp_dir = tempfile::tempdir()?;
    let archive_path = tmp_dir.path().join("addon.zip");
    state.forge.download_file(link, &archive_path).await?;

    let dirs = QumaDirs::from_legacy(state.spt_dir.clone());
    let staging_dir = crate::ops::staging_tempdir(&dirs)?;
    let staging_path = staging_dir.path().to_path_buf();

    let config = state.config_cloned();
    let version_str = version.version.clone();
    let mod_version_constraint = version.mod_version_constraint.clone();
    let forge_addon_id = op
        .forge_addon_id
        .ok_or_else(|| anyhow::anyhow!("addon operation missing forge_addon_id"))?;

    let staging_path2 = staging_path.clone();
    let extracted =
        web::block(move || crate::spt::mods::extract_mod(&archive_path, &staging_path2))
            .await
            .map_err(|e| anyhow::anyhow!("{e}"))??;

    // Update
    crate::ops::apply_addon_update(
        state.db.clone(),
        dirs,
        config,
        staging_path,
        extracted,
        addon_db_id,
        version_id,
        version_str,
        mod_version_constraint,
        forge_addon_id,
    )
    .await?;

    Ok(())
}

async fn apply_addon_remove(op: &PendingOperation, state: &AppState) -> anyhow::Result<()> {
    let forge_addon_id = op
        .forge_addon_id
        .ok_or_else(|| anyhow::anyhow!("addon operation missing forge_addon_id"))?;

    let db = state.db.clone();
    let dirs = QumaDirs::from_legacy(state.spt_dir.clone());
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
