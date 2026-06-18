use actix_web::web::Data;
use actix_web::HttpResponse;

use crate::podman::PodmanClient;
use crate::web::error::WebError;
use crate::web::state::AppState;

pub async fn start_server(state: Data<AppState>) -> actix_web::Result<HttpResponse> {
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

pub async fn stop_server(state: Data<AppState>) -> actix_web::Result<HttpResponse> {
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

pub async fn restart_server(state: Data<AppState>) -> actix_web::Result<HttpResponse> {
    let container = state
        .config
        .server_container
        .as_deref()
        .ok_or(WebError::BadRequest(
            "No server_container configured".to_string(),
        ))?;

    let podman = PodmanClient::new(container);
    podman.stop().await.map_err(WebError::from)?;
    podman.start().await.map_err(WebError::from)?;

    Ok(HttpResponse::SeeOther()
        .insert_header(("Location", "/status"))
        .finish())
}
