use actix_session::Session;
use actix_web::web::{self, Data, Form};
use actix_web::{HttpRequest, HttpResponse};

use crate::db::users::Role;
use crate::web::auth::{require_auth, require_capability};
use crate::web::error::WebError;
use crate::web::flash::set_flash;
use crate::web::sse::ServerEvent;
use crate::web::state::AppState;

pub async fn start_server(
    state: Data<AppState>,
    req: HttpRequest,
    session: Session,
    form: Form<crate::web::csrf::CsrfForm>,
) -> actix_web::Result<HttpResponse> {
    let user = require_auth(&req)?;
    require_capability(&user, Role::can_control_server)?;
    if !crate::web::csrf::validate_token(&session, &form.csrf_token) {
        return Err(WebError::Forbidden.into());
    }
    let container = match state.config.server_container.as_deref() {
        Some(c) => c,
        None => {
            set_flash(
                &session,
                "No server_container configured. Set it in quartermaster.toml.",
                "error",
            );
            return Ok(HttpResponse::SeeOther()
                .insert_header(("Location", "/quma/status"))
                .finish());
        }
    };

    let mgr = match state.container_mgr.as_ref() {
        Some(m) => m,
        None => {
            set_flash(
                &session,
                "Podman socket not available. Ensure podman.socket is enabled.",
                "error",
            );
            return Ok(HttpResponse::SeeOther()
                .insert_header(("Location", "/quma/status"))
                .finish());
        }
    };

    state.set_server_transition(Some("starting"));
    let _ = state.events.send(ServerEvent::ServerTransition);

    if let Err(e) = mgr.start(container).await {
        tracing::error!(container, error = %e, "failed to start server");
        set_flash(&session, &format!("Failed to start server: {e}"), "error");
    } else {
        tracing::info!(container, "server started");
        set_flash(&session, "Server starting", "success");
    }

    state.set_server_transition(None);
    let _ = state.events.send(ServerEvent::ServerTransition);

    Ok(HttpResponse::SeeOther()
        .insert_header(("Location", "/quma/status"))
        .finish())
}

pub async fn stop_server(
    state: Data<AppState>,
    req: HttpRequest,
    session: Session,
    form: Form<crate::web::csrf::CsrfForm>,
) -> actix_web::Result<HttpResponse> {
    let user = require_auth(&req)?;
    require_capability(&user, Role::can_control_server)?;
    if !crate::web::csrf::validate_token(&session, &form.csrf_token) {
        return Err(WebError::Forbidden.into());
    }
    let container = match state.config.server_container.as_deref() {
        Some(c) => c,
        None => {
            set_flash(
                &session,
                "No server_container configured. Set it in quartermaster.toml.",
                "error",
            );
            return Ok(HttpResponse::SeeOther()
                .insert_header(("Location", "/quma/status"))
                .finish());
        }
    };

    let mgr = match state.container_mgr.as_ref() {
        Some(m) => m,
        None => {
            set_flash(
                &session,
                "Podman socket not available. Ensure podman.socket is enabled.",
                "error",
            );
            return Ok(HttpResponse::SeeOther()
                .insert_header(("Location", "/quma/status"))
                .finish());
        }
    };

    state.set_server_transition(Some("stopping"));
    let _ = state.events.send(ServerEvent::ServerTransition);

    if let Err(e) = mgr.stop(container).await {
        tracing::error!(container, error = %e, "failed to stop server");
        set_flash(&session, &format!("Failed to stop server: {e}"), "error");
    } else {
        tracing::info!(container, "server stopped");
        set_flash(&session, "Server stopped", "success");
    }

    state.set_server_transition(None);
    let _ = state.events.send(ServerEvent::ServerTransition);

    Ok(HttpResponse::SeeOther()
        .insert_header(("Location", "/quma/status"))
        .finish())
}

pub async fn restart_server(
    state: Data<AppState>,
    req: HttpRequest,
    session: Session,
    form: Form<crate::web::csrf::CsrfForm>,
) -> actix_web::Result<HttpResponse> {
    let user = require_auth(&req)?;
    require_capability(&user, Role::can_control_server)?;
    if !crate::web::csrf::validate_token(&session, &form.csrf_token) {
        return Err(WebError::Forbidden.into());
    }
    let container = match state.config.server_container.as_deref() {
        Some(c) => c,
        None => {
            set_flash(
                &session,
                "No server_container configured. Set it in quartermaster.toml.",
                "error",
            );
            return Ok(HttpResponse::SeeOther()
                .insert_header(("Location", "/quma/status"))
                .finish());
        }
    };

    let mgr = match state.container_mgr.as_ref() {
        Some(m) => m,
        None => {
            set_flash(
                &session,
                "Podman socket not available. Ensure podman.socket is enabled.",
                "error",
            );
            return Ok(HttpResponse::SeeOther()
                .insert_header(("Location", "/quma/status"))
                .finish());
        }
    };

    state.set_server_transition(Some("restarting"));
    let _ = state.events.send(ServerEvent::ServerTransition);

    // Stop first
    if let Err(e) = mgr.stop(container).await {
        tracing::error!(container, error = %e, "failed to stop server for restart");
        set_flash(&session, &format!("Failed to stop server: {e}"), "error");
        state.set_server_transition(None);
        let _ = state.events.send(ServerEvent::ServerTransition);
        return Ok(HttpResponse::SeeOther()
            .insert_header(("Location", "/quma/status"))
            .finish());
    }

    // Drain queue if configured
    if state.config.auto_drain_on_lifecycle {
        let db = state.db.clone();
        let ops = web::block(move || {
            let db = db.lock();
            db.list_pending_ops()
        })
        .await
        .map_err(WebError::from)?
        .map_err(WebError::from)?;

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

    // Start
    if let Err(e) = mgr.start(container).await {
        tracing::error!(container, error = %e, "failed to start server after restart");
        set_flash(
            &session,
            &format!("Server stopped but failed to start: {e}"),
            "error",
        );
    } else {
        tracing::info!(container, "server restarted");
        set_flash(&session, "Server restarted", "success");
    }

    state.set_server_transition(None);
    let _ = state.events.send(ServerEvent::ServerTransition);

    Ok(HttpResponse::SeeOther()
        .insert_header(("Location", "/quma/status"))
        .finish())
}
