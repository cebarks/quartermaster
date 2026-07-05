use std::sync::Arc;

use actix_session::Session;
use actix_web::web::{self, Data, Form};
use actix_web::{HttpRequest, HttpResponse};

use crate::container::ContainerManager;
use crate::db::rbac::Permission;
use crate::web::auth::{require_auth, require_permission};
use crate::web::error::WebError;
use crate::web::flash::{set_flash, FlashType};
use crate::web::sse::ServerEvent;
use crate::web::state::AppState;

/// Validates that both a container name and container manager are configured.
/// Returns a redirect to `/quma/` with an appropriate flash error if either is missing.
fn require_container<'a>(
    state: &'a AppState,
    session: &Session,
) -> Result<(String, &'a Arc<ContainerManager>), HttpResponse> {
    let name = state.config().server_container.clone().ok_or_else(|| {
        set_flash(
            session,
            "No server_container configured. Set it in quartermaster.toml.",
            FlashType::Error,
        );
        HttpResponse::SeeOther()
            .insert_header(("Location", "/quma/"))
            .finish()
    })?;

    let mgr = state.container_mgr.as_ref().ok_or_else(|| {
        set_flash(
            session,
            "Podman socket not available. Ensure podman.socket is enabled.",
            FlashType::Error,
        );
        HttpResponse::SeeOther()
            .insert_header(("Location", "/quma/"))
            .finish()
    })?;

    Ok((name, mgr))
}

pub async fn start_server(
    state: Data<AppState>,
    req: HttpRequest,
    session: Session,
    form: Form<crate::web::csrf::CsrfForm>,
) -> actix_web::Result<HttpResponse> {
    let user = require_auth(&req)?;
    require_permission(&user, Permission::ServerControl)?;
    if !crate::web::csrf::validate_token(&session, &form.csrf_token) {
        return Err(WebError::Forbidden.into());
    }
    let (container, mgr) = match require_container(&state, &session) {
        Ok(pair) => pair,
        Err(resp) => return Ok(resp),
    };

    state.set_server_transition(Some("starting"));
    let _ = state.events.send(ServerEvent::ServerTransition);

    if let Err(e) = mgr.start(&container).await {
        tracing::error!(container, err = %e, "failed to start server");
        set_flash(
            &session,
            &format!("Failed to start server: {e}"),
            FlashType::Error,
        );
    } else {
        tracing::info!(container, "server started");
        if let Some(ref svm) = state.svm {
            svm.write().clear_dirty();
        }
        set_flash(&session, "Server starting", FlashType::Success);
    }

    state.set_server_transition(None);
    let _ = state.events.send(ServerEvent::ServerTransition);

    Ok(HttpResponse::SeeOther()
        .insert_header(("Location", "/quma/"))
        .finish())
}

pub async fn stop_server(
    state: Data<AppState>,
    req: HttpRequest,
    session: Session,
    form: Form<crate::web::csrf::CsrfForm>,
) -> actix_web::Result<HttpResponse> {
    let user = require_auth(&req)?;
    require_permission(&user, Permission::ServerControl)?;
    if !crate::web::csrf::validate_token(&session, &form.csrf_token) {
        return Err(WebError::Forbidden.into());
    }
    let (container, mgr) = match require_container(&state, &session) {
        Ok(pair) => pair,
        Err(resp) => return Ok(resp),
    };

    state.set_server_transition(Some("stopping"));
    let _ = state.events.send(ServerEvent::ServerTransition);

    if let Err(e) = mgr.stop(&container).await {
        tracing::error!(container, err = %e, "failed to stop server");
        set_flash(
            &session,
            &format!("Failed to stop server: {e}"),
            FlashType::Error,
        );
    } else {
        tracing::info!(container, "server stopped");
        set_flash(&session, "Server stopped", FlashType::Success);
    }

    state.set_server_transition(None);
    let _ = state.events.send(ServerEvent::ServerTransition);

    Ok(HttpResponse::SeeOther()
        .insert_header(("Location", "/quma/"))
        .finish())
}

pub async fn restart_server(
    state: Data<AppState>,
    req: HttpRequest,
    session: Session,
    form: Form<crate::web::csrf::CsrfForm>,
) -> actix_web::Result<HttpResponse> {
    let user = require_auth(&req)?;
    require_permission(&user, Permission::ServerControl)?;
    if !crate::web::csrf::validate_token(&session, &form.csrf_token) {
        return Err(WebError::Forbidden.into());
    }
    let (container, mgr) = match require_container(&state, &session) {
        Ok(pair) => pair,
        Err(resp) => return Ok(resp),
    };

    state.set_server_transition(Some("restarting"));
    let _ = state.events.send(ServerEvent::ServerTransition);

    // Stop first
    if let Err(e) = mgr.stop(&container).await {
        tracing::error!(container, err = %e, "failed to stop server for restart");
        set_flash(
            &session,
            &format!("Failed to stop server: {e}"),
            FlashType::Error,
        );
        state.set_server_transition(None);
        let _ = state.events.send(ServerEvent::ServerTransition);
        return Ok(HttpResponse::SeeOther()
            .insert_header(("Location", "/quma/"))
            .finish());
    }

    // Drain queue if configured. Errors here are non-fatal — the server is
    // already stopped, so we log the failure and continue to the start phase.
    if state.config().auto_drain_on_lifecycle {
        let db = state.db.clone();
        match web::block(move || {
            let db = db.lock();
            db.list_pending_ops()
        })
        .await
        {
            Ok(Ok(ops)) => {
                for op in &ops {
                    let result = match op.action {
                        crate::db::users::QueueAction::Install => {
                            super::queue::apply_install(op, &state).await
                        }
                        crate::db::users::QueueAction::Update => {
                            super::queue::apply_update(op, &state).await
                        }
                        crate::db::users::QueueAction::Remove => {
                            super::queue::apply_remove(op, &state).await
                        }
                    };
                    if result.is_ok() {
                        crate::queue::cleanup_queued_archive(op);
                        let db = state.db.clone();
                        let op_id = op.id;
                        let _ = web::block(move || {
                            let db = db.lock();
                            db.delete_pending_op(op_id)
                        })
                        .await;
                    }
                }
            }
            Ok(Err(e)) => {
                tracing::error!(err = %e, "failed to list pending ops during restart queue drain");
            }
            Err(e) => {
                tracing::error!(err = %e, "blocking error during restart queue drain");
            }
        }
    }

    // Start
    if let Err(e) = mgr.start(&container).await {
        tracing::error!(container, err = %e, "failed to start server after restart");
        set_flash(
            &session,
            &format!("Server stopped but failed to start: {e}"),
            FlashType::Error,
        );
    } else {
        tracing::info!(container, "server restarted");
        if let Some(ref svm) = state.svm {
            svm.write().clear_dirty();
        }
        set_flash(&session, "Server restarted", FlashType::Success);
    }

    state.set_server_transition(None);
    let _ = state.events.send(ServerEvent::ServerTransition);

    Ok(HttpResponse::SeeOther()
        .insert_header(("Location", "/quma/"))
        .finish())
}
