use actix_session::Session;
use actix_web::web::{self, Data};
use actix_web::HttpResponse;

use crate::podman::PodmanClient;
use crate::web::auth::require_admin;
use crate::web::error::WebError;
use crate::web::state::AppState;

pub async fn start_server(
    state: Data<AppState>,
    session: Session,
) -> actix_web::Result<HttpResponse> {
    require_admin(&session)?;
    let container = state
        .config
        .server_container
        .as_deref()
        .ok_or(WebError::BadRequest(
            "No server_container configured".to_string(),
        ))?;

    let podman = PodmanClient::new(container);
    podman.start().await.map_err(WebError::from)?;

    Ok(HttpResponse::SeeOther()
        .insert_header(("Location", "/status"))
        .finish())
}

pub async fn stop_server(
    state: Data<AppState>,
    session: Session,
) -> actix_web::Result<HttpResponse> {
    require_admin(&session)?;
    let container = state
        .config
        .server_container
        .as_deref()
        .ok_or(WebError::BadRequest(
            "No server_container configured".to_string(),
        ))?;

    let podman = PodmanClient::new(container);
    podman.stop().await.map_err(WebError::from)?;

    Ok(HttpResponse::SeeOther()
        .insert_header(("Location", "/status"))
        .finish())
}

pub async fn restart_server(
    state: Data<AppState>,
    session: Session,
) -> actix_web::Result<HttpResponse> {
    require_admin(&session)?;
    let container = state
        .config
        .server_container
        .as_deref()
        .ok_or(WebError::BadRequest(
            "No server_container configured".to_string(),
        ))?;

    let podman = PodmanClient::new(container);
    podman.stop().await.map_err(WebError::from)?;

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
            let result = match op.action.as_str() {
                "install" => super::queue::apply_install(op, &state).await,
                "update" => super::queue::apply_update(op, &state).await,
                "remove" => super::queue::apply_remove(op, &state).await,
                _ => Ok(()),
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

    podman.start().await.map_err(WebError::from)?;

    Ok(HttpResponse::SeeOther()
        .insert_header(("Location", "/status"))
        .finish())
}
