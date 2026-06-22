use actix_session::Session;
use actix_web::web::{self, Data, Form, Html, Path};
use actix_web::{HttpRequest, HttpResponse};
use askama::Template;

use crate::db::users::{PendingOperation, Role};
use crate::web::auth::{require_auth, require_capability, SessionUser};
use crate::web::error::WebError;
use crate::web::flash::{set_flash, FlashType};
use crate::web::state::AppState;

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

pub async fn cancel_op(
    state: Data<AppState>,
    path: Path<i64>,
    req: HttpRequest,
    session: Session,
    form: Form<crate::web::csrf::CsrfForm>,
) -> actix_web::Result<HttpResponse> {
    let user = require_auth(&req)?;
    require_capability(&user, Role::can_manage_queue)?;
    if !crate::web::csrf::validate_token(&session, &form.csrf_token) {
        return Err(WebError::Forbidden.into());
    }
    let op_id = path.into_inner();
    let db = state.db.clone();

    web::block(move || {
        let db = db.lock();
        db.delete_pending_op(op_id)
    })
    .await
    .map_err(WebError::from)?
    .map_err(WebError::from)?;

    set_flash(&session, "Operation cancelled", FlashType::Success);
    Ok(HttpResponse::SeeOther()
        .insert_header(("Location", "/quma/mods"))
        .finish())
}

pub async fn apply_queue(
    state: Data<AppState>,
    req: HttpRequest,
    session: Session,
    form: Form<crate::web::csrf::CsrfForm>,
) -> actix_web::Result<HttpResponse> {
    let user = require_auth(&req)?;
    require_capability(&user, Role::can_manage_queue)?;
    if !crate::web::csrf::validate_token(&session, &form.csrf_token) {
        return Err(WebError::Forbidden.into());
    }
    let server_running = crate::server_detect::is_server_running(
        &state.config,
        &state.spt_dir,
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
            .insert_header(("Location", "/quma/mods"))
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

    let mut failures: Vec<String> = Vec::new();

    for op in &ops {
        let result = match op.action {
            crate::db::users::QueueAction::Install => apply_install(op, &state).await,
            crate::db::users::QueueAction::Update => apply_update(op, &state).await,
            crate::db::users::QueueAction::Remove => apply_remove(op, &state).await,
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
                tracing::error!(action = %op.action, mod_name = %op.mod_name, error = %e, "queue apply failed");
                failures.push(op.mod_name.clone());
            }
        }
    }

    // Regenerate NarcoNet config after all queue operations
    {
        let db = state.db.clone();
        let spt_dir = state.spt_dir.clone();
        let config = state.config.clone();
        let _ = web::block(move || {
            let db = db.lock();
            crate::modsync::regenerate_if_enabled(&spt_dir, &config, &db)
        })
        .await;
    }

    if !failures.is_empty() {
        let names = failures.join(", ");
        let msg = format!("{} operation(s) failed: {names}", failures.len());
        set_flash(&session, &msg, FlashType::Error);
        return Ok(HttpResponse::SeeOther()
            .insert_header(("Location", "/quma/mods"))
            .finish());
    }

    set_flash(&session, "Queue applied successfully", FlashType::Success);
    Ok(HttpResponse::SeeOther()
        .insert_header(("Location", "/quma/mods"))
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
    let config = state.config.clone();
    let mod_name = op.mod_name.clone();
    let forge_mod_id = op.forge_mod_id;

    web::block(move || {
        let db = db.lock();
        if db.get_mod_by_forge_id(forge_mod_id)?.is_some() {
            return Ok(());
        }
        crate::ops::install_mod_from_archive(&crate::ops::InstallRequest {
            db: &db,
            spt_dir: &spt_dir,
            config: &config,
            forge_mod_id,
            version_id,
            name: &mod_name,
            slug: None,
            version: &version_str,
            archive_path: &archive_path,
        })?;
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
    let tmp_dir = download_to_temp(state, &link).await?;
    let archive_path = tmp_dir.path().join("mod.zip");

    let db = state.db.clone();
    let spt_dir = state.spt_dir.clone();
    let config = state.config.clone();
    let forge_mod_id = op.forge_mod_id;

    web::block(move || {
        let db = db.lock();
        let installed = db
            .get_mod_by_forge_id(forge_mod_id)?
            .ok_or_else(|| anyhow::anyhow!("mod not found for forge_id {forge_mod_id}"))?;
        crate::ops::update_mod_from_archive(
            &db,
            &spt_dir,
            &config,
            installed.id,
            version_id,
            &version_str,
            &archive_path,
        )
    })
    .await??;

    Ok(())
}

pub(super) async fn apply_remove(op: &PendingOperation, state: &AppState) -> anyhow::Result<()> {
    let db = state.db.clone();
    let spt_dir = state.spt_dir.clone();
    let config = state.config.clone();
    let forge_mod_id = op.forge_mod_id;

    web::block(move || {
        let db = db.lock();
        let installed = db
            .get_mod_by_forge_id(forge_mod_id)?
            .ok_or_else(|| anyhow::anyhow!("mod not found for forge_id {forge_mod_id}"))?;

        // Collect and remove reverse dependencies (same as CLI drain_all)
        let reverse_deps = crate::ops::collect_all_reverse_deps(&db, installed.id)?;
        for dep in reverse_deps.iter().rev() {
            crate::ops::remove_mod_by_id(&db, &spt_dir, &config, dep.id)?;
        }

        crate::ops::remove_mod_by_id(&db, &spt_dir, &config, installed.id)
    })
    .await??;

    Ok(())
}
