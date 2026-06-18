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

    let mut failures: Vec<String> = Vec::new();

    for op in &ops {
        let result = match op.action.as_str() {
            "install" => apply_install(op, &state).await,
            "update" => apply_update(op, &state).await,
            "remove" => apply_remove(op, &state).await,
            _ => Ok(()),
        };

        match result {
            Ok(()) => {
                let db = state.db.clone();
                let op_id = op.id;
                web::block(move || {
                    let db = db.lock();
                    db.delete_pending_op(op_id)
                })
                .await
                .map_err(WebError::from)?
                .map_err(WebError::from)?;
            }
            Err(e) => {
                eprintln!(
                    "queue apply failed for {} '{}': {e}",
                    op.action, op.mod_name
                );
                failures.push(format!("{} '{}': {e}", op.action, op.mod_name));
            }
        }
    }

    if !failures.is_empty() {
        let msg = format!(
            "{} operation(s) failed and remain in queue:\n{}",
            failures.len(),
            failures.join("\n")
        );
        return Ok(HttpResponse::InternalServerError().body(msg));
    }

    Ok(HttpResponse::SeeOther()
        .insert_header(("Location", "/queue"))
        .finish())
}

async fn resolve_version_link(
    state: &AppState,
    forge_mod_id: i64,
    version_id: i64,
) -> anyhow::Result<(String, String)> {
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
    Ok((link.to_string(), version.version.clone()))
}

async fn download_to_temp(state: &AppState, link: &str) -> anyhow::Result<tempfile::TempDir> {
    let tmp_dir = tempfile::tempdir()?;
    let archive_path = tmp_dir.path().join("mod.zip");
    state.forge.download_file(link, &archive_path).await?;
    Ok(tmp_dir)
}

pub(super) async fn apply_install(op: &PendingOperation, state: &AppState) -> anyhow::Result<()> {
    let version_id = op
        .forge_version_id
        .ok_or_else(|| anyhow::anyhow!("install op missing version_id"))?;
    let (link, version_str) = resolve_version_link(state, op.forge_mod_id, version_id).await?;
    let tmp_dir = download_to_temp(state, &link).await?;
    let archive_path = tmp_dir.path().join("mod.zip");

    let db = state.db.clone();
    let spt_dir = state.spt_dir.clone();
    let mod_name = op.mod_name.clone();
    let forge_mod_id = op.forge_mod_id;

    web::block(move || {
        use crate::spt::mods::extract_mod;
        let extracted = extract_mod(&archive_path, &spt_dir)?;
        let db = db.lock();
        let installed_id =
            db.insert_mod(forge_mod_id, version_id, &mod_name, None, &version_str)?;
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
    .await??;

    Ok(())
}

pub(super) async fn apply_update(op: &PendingOperation, state: &AppState) -> anyhow::Result<()> {
    let version_id = op
        .forge_version_id
        .ok_or_else(|| anyhow::anyhow!("update op missing version_id"))?;
    let (link, version_str) = resolve_version_link(state, op.forge_mod_id, version_id).await?;

    // Extract new version to staging BEFORE deleting old files
    let tmp_dir = download_to_temp(state, &link).await?;
    let archive_path = tmp_dir.path().join("mod.zip");
    let staging_dir = tempfile::tempdir()?;
    let staging_path = staging_dir.path().to_path_buf();

    let archive_for_staging = archive_path.clone();
    let staging_for_extract = staging_path.clone();
    let extracted = web::block(move || {
        crate::spt::mods::extract_mod(&archive_for_staging, &staging_for_extract)
    })
    .await??;

    // Now safe to delete old and move new into place
    let db = state.db.clone();
    let spt_dir = state.spt_dir.clone();
    let forge_mod_id = op.forge_mod_id;

    web::block(move || {
        use crate::spt::mods::delete_mod_files;

        let db = db.lock();
        let installed = db
            .get_mod_by_forge_id(forge_mod_id)?
            .ok_or_else(|| anyhow::anyhow!("mod not found for forge_id {forge_mod_id}"))?;

        let old_files = db.get_files_for_mod(installed.id)?;
        let old_paths: Vec<String> = old_files.iter().map(|f| f.file_path.clone()).collect();
        delete_mod_files(&spt_dir, &old_paths)?;
        db.delete_files_for_mod(installed.id)?;

        // Move staged files into real location
        for file in &extracted {
            let src = staging_path.join(&file.path);
            let dst = spt_dir.join(&file.path);
            if let Some(parent) = dst.parent() {
                std::fs::create_dir_all(parent)?;
            }
            if src.exists() {
                std::fs::rename(&src, &dst).or_else(|_| std::fs::copy(&src, &dst).map(|_| ()))?;
            }
        }

        for file in &extracted {
            db.insert_file(
                installed.id,
                &file.path,
                Some(&file.hash),
                Some(file.size as i64),
            )?;
        }
        db.update_mod(installed.id, version_id, &version_str)?;
        Ok::<_, anyhow::Error>(())
    })
    .await??;

    Ok(())
}

pub(super) async fn apply_remove(op: &PendingOperation, state: &AppState) -> anyhow::Result<()> {
    let db = state.db.clone();
    let spt_dir = state.spt_dir.clone();
    let forge_mod_id = op.forge_mod_id;

    web::block(move || {
        use crate::spt::mods::delete_mod_files;
        let db = db.lock();
        let installed = db
            .get_mod_by_forge_id(forge_mod_id)?
            .ok_or_else(|| anyhow::anyhow!("mod not found for forge_id {forge_mod_id}"))?;
        let files = db.get_files_for_mod(installed.id)?;
        let paths: Vec<String> = files.iter().map(|f| f.file_path.clone()).collect();
        delete_mod_files(&spt_dir, &paths)?;
        db.delete_mod(installed.id)?;
        Ok::<_, anyhow::Error>(())
    })
    .await??;

    Ok(())
}
