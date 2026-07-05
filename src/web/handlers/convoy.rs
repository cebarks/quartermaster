use actix_web::{web, HttpRequest, HttpResponse};
use serde::Deserialize;

use crate::web::state::AppState;

/// GET /quma/convoy/catalog — serve cached catalog JSON with ETag
pub async fn catalog(
    req: HttpRequest,
    state: web::Data<AppState>,
) -> actix_web::Result<HttpResponse> {
    let Some((path, etag)) = state.catalog_cache.get() else {
        return Ok(HttpResponse::ServiceUnavailable()
            .body("Convoy catalog is being built, try again shortly"));
    };

    // Check If-None-Match for 304
    if let Some(if_none_match) = req.headers().get("if-none-match") {
        if let Ok(val) = if_none_match.to_str() {
            if val == etag {
                return Ok(HttpResponse::NotModified().finish());
            }
        }
    }

    let body = web::block(move || std::fs::read(path))
        .await
        .map_err(actix_web::error::ErrorInternalServerError)?
        .map_err(actix_web::error::ErrorInternalServerError)?;

    Ok(HttpResponse::Ok()
        .content_type("application/json")
        .insert_header(("etag", etag))
        .insert_header(("cache-control", "no-cache"))
        .body(body))
}

#[derive(Deserialize)]
pub struct DownloadRequest {
    pub mods: Vec<i64>,
}

/// POST /quma/convoy/download — batched mod archive download
pub async fn download(
    state: web::Data<AppState>,
    body: web::Json<DownloadRequest>,
) -> actix_web::Result<HttpResponse> {
    if body.mods.is_empty() {
        return Ok(HttpResponse::BadRequest().body("no mods requested"));
    }

    let db = state.db.clone();
    let spt_dir = state.spt_dir.clone();
    let forge_ids = body.mods.clone();

    let zip_bytes = web::block(move || {
        let db = db.lock();
        crate::convoy::download::build_download_zip(&db, &spt_dir, &forge_ids)
    })
    .await
    .map_err(actix_web::error::ErrorInternalServerError)?
    .map_err(actix_web::error::ErrorInternalServerError)?;

    Ok(HttpResponse::Ok()
        .content_type("application/zip")
        .insert_header((
            "content-disposition",
            "attachment; filename=\"convoy-mods.zip\"",
        ))
        .body(zip_bytes))
}

/// GET /quma/convoy/mod/{forge_id}/archive — single mod download
pub async fn single_mod_archive(
    state: web::Data<AppState>,
    path: web::Path<i64>,
) -> actix_web::Result<HttpResponse> {
    let forge_id = path.into_inner();
    let db = state.db.clone();
    let spt_dir = state.spt_dir.clone();

    let zip_bytes = web::block(move || {
        let db = db.lock();
        crate::convoy::download::build_download_zip(&db, &spt_dir, &[forge_id])
    })
    .await
    .map_err(actix_web::error::ErrorInternalServerError)?
    .map_err(actix_web::error::ErrorInternalServerError)?;

    Ok(HttpResponse::Ok()
        .content_type("application/zip")
        .insert_header((
            "content-disposition",
            format!("attachment; filename=\"mod-{forge_id}.zip\""),
        ))
        .body(zip_bytes))
}
