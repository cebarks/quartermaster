use actix_web::web::Bytes;
use actix_web::{web, HttpResponse};
use futures_util::StreamExt;
use serde::{Deserialize, Serialize};

use crate::headless::service::LifecycleAction;
use crate::web::state::AppState;

// Request body types
#[derive(Deserialize)]
pub struct ScaleRequest {
    pub count: u32,
    #[serde(default)]
    pub force: bool,
}

#[derive(Deserialize)]
pub struct DeleteRequest {
    #[serde(default)]
    pub force: bool,
}

#[derive(Deserialize)]
pub struct RebuildRequest {
    #[serde(default)]
    pub force: bool,
}

#[derive(Deserialize)]
pub struct RenameRequest {
    pub name: String,
}

#[derive(Deserialize)]
pub struct ImageRequest {
    pub image: String,
}

#[derive(Deserialize)]
pub struct StartRaidRequest {
    pub location_id: String,
    pub time: i32,
    #[serde(default)]
    pub use_event: bool,
}

#[derive(Deserialize)]
pub struct LogsQuery {
    #[serde(default = "default_tail")]
    pub tail: usize,
    #[serde(default)]
    pub follow: bool,
}

fn default_tail() -> usize {
    100
}

// Response types
#[derive(Serialize)]
pub struct OkResponse {
    pub ok: bool,
}

#[derive(Serialize)]
pub struct OperationIdResponse {
    pub operation_id: u64,
}

#[derive(Serialize)]
pub struct GracefulRestartResponse {
    pub result: String,
}

#[derive(Serialize)]
pub struct ErrorResponse {
    pub error: String,
}

// Handler: GET /quma/api/headless/status
pub async fn api_headless_status(state: web::Data<AppState>) -> actix_web::Result<HttpResponse> {
    let service = state.headless_service()?;
    let states = service.status().await;
    Ok(HttpResponse::Ok().json(states))
}

// Handler: POST /quma/api/headless/{n}/start
pub async fn api_client_start(
    state: web::Data<AppState>,
    path: web::Path<u32>,
) -> actix_web::Result<HttpResponse> {
    let service = state.headless_service()?;
    service
        .client_lifecycle(path.into_inner(), LifecycleAction::Start)
        .await?;
    Ok(HttpResponse::Ok().json(OkResponse { ok: true }))
}

// Handler: POST /quma/api/headless/{n}/stop
pub async fn api_client_stop(
    state: web::Data<AppState>,
    path: web::Path<u32>,
) -> actix_web::Result<HttpResponse> {
    let service = state.headless_service()?;
    service
        .client_lifecycle(path.into_inner(), LifecycleAction::Stop)
        .await?;
    Ok(HttpResponse::Ok().json(OkResponse { ok: true }))
}

// Handler: POST /quma/api/headless/{n}/restart
pub async fn api_client_restart(
    state: web::Data<AppState>,
    path: web::Path<u32>,
) -> actix_web::Result<HttpResponse> {
    let service = state.headless_service()?;
    service
        .client_lifecycle(path.into_inner(), LifecycleAction::Restart)
        .await?;
    Ok(HttpResponse::Ok().json(OkResponse { ok: true }))
}

// Handler: POST /quma/api/headless/{n}/graceful-restart
pub async fn api_client_graceful_restart(
    state: web::Data<AppState>,
    path: web::Path<u32>,
) -> actix_web::Result<HttpResponse> {
    let service = state.headless_service()?;
    let result = service.graceful_restart(path.into_inner()).await?;
    let result_str = match result {
        crate::headless::service::GracefulResult::Exited => "exited",
        crate::headless::service::GracefulResult::Timeout => "timeout",
    };
    Ok(HttpResponse::Ok().json(GracefulRestartResponse {
        result: result_str.to_string(),
    }))
}

// Handler: POST /quma/api/headless/scale
pub async fn api_scale(
    state: web::Data<AppState>,
    body: web::Json<ScaleRequest>,
) -> actix_web::Result<HttpResponse> {
    let service = state.headless_service()?;
    let op_id = service.scale(body.count, body.force).await?;
    Ok(HttpResponse::Ok().json(OperationIdResponse {
        operation_id: op_id.0,
    }))
}

// Handler: POST /quma/api/headless/create
pub async fn api_create(state: web::Data<AppState>) -> actix_web::Result<HttpResponse> {
    let service = state.headless_service()?;
    let op_id = service.create().await?;
    Ok(HttpResponse::Ok().json(OperationIdResponse {
        operation_id: op_id.0,
    }))
}

// Handler: POST /quma/api/headless/{n}/delete
pub async fn api_client_delete(
    state: web::Data<AppState>,
    path: web::Path<u32>,
    body: web::Json<DeleteRequest>,
) -> actix_web::Result<HttpResponse> {
    let service = state.headless_service()?;
    let op_id = service.delete(path.into_inner(), body.force).await?;
    Ok(HttpResponse::Ok().json(OperationIdResponse {
        operation_id: op_id.0,
    }))
}

// Handler: POST /quma/api/headless/rebuild
pub async fn api_rebuild(
    state: web::Data<AppState>,
    body: web::Json<RebuildRequest>,
) -> actix_web::Result<HttpResponse> {
    let service = state.headless_service()?;
    let op_id = service.rebuild(body.force).await?;
    Ok(HttpResponse::Ok().json(OperationIdResponse {
        operation_id: op_id.0,
    }))
}

// Handler: POST /quma/api/headless/converge
pub async fn api_converge(state: web::Data<AppState>) -> actix_web::Result<HttpResponse> {
    let service = state.headless_service()?;
    let op_id = service.converge().await?;
    Ok(HttpResponse::Ok().json(OperationIdResponse {
        operation_id: op_id.0,
    }))
}

// Handler: POST /quma/api/headless/{n}/rename
pub async fn api_client_rename(
    state: web::Data<AppState>,
    path: web::Path<u32>,
    body: web::Json<RenameRequest>,
) -> actix_web::Result<HttpResponse> {
    let service = state.headless_service()?;
    service.rename(path.into_inner(), &body.name).await?;
    Ok(HttpResponse::Ok().json(OkResponse { ok: true }))
}

// Handler: POST /quma/api/headless/{n}/image
pub async fn api_client_set_image(
    state: web::Data<AppState>,
    path: web::Path<u32>,
    body: web::Json<ImageRequest>,
) -> actix_web::Result<HttpResponse> {
    let service = state.headless_service()?;
    service
        .set_image(path.into_inner(), Some(body.image.clone()))
        .await?;
    Ok(HttpResponse::Ok().json(OkResponse { ok: true }))
}

// Handler: POST /quma/api/headless/{n}/start-raid
pub async fn api_client_start_raid(
    state: web::Data<AppState>,
    path: web::Path<u32>,
    body: web::Json<StartRaidRequest>,
) -> actix_web::Result<HttpResponse> {
    let service = state.headless_service()?;
    service
        .start_raid(
            path.into_inner(),
            &body.location_id,
            body.time,
            body.use_event,
        )
        .await?;
    Ok(HttpResponse::Ok().json(OkResponse { ok: true }))
}

// Handler: GET /quma/api/headless/{n}/logs
pub async fn api_client_logs(
    state: web::Data<AppState>,
    path: web::Path<u32>,
    query: web::Query<LogsQuery>,
) -> actix_web::Result<HttpResponse> {
    let service = state.headless_service()?;
    let stream = service.logs(path.into_inner(), query.tail, query.follow);

    Ok(HttpResponse::Ok()
        .content_type("text/event-stream")
        .insert_header(("Cache-Control", "no-cache"))
        .insert_header(("Connection", "keep-alive"))
        .streaming(stream.map(|line| {
            let json = serde_json::to_string(&line).unwrap_or_default();
            Ok::<_, actix_web::Error>(Bytes::from(format!("data: {json}\n\n")))
        })))
}

// Handler: GET /quma/api/headless/operations/{id}
pub async fn api_operation_status(
    state: web::Data<AppState>,
    path: web::Path<u64>,
) -> actix_web::Result<HttpResponse> {
    let service = state.headless_service()?;
    match service.operations().poll(path.into_inner()) {
        Some(status) => Ok(HttpResponse::Ok().json(status)),
        None => Ok(HttpResponse::NotFound().json(ErrorResponse {
            error: "operation_not_found".to_string(),
        })),
    }
}
