use actix_web::{web, HttpResponse};
use askama::Template;

use crate::config::NARCONET_FORGE_MOD_ID;
use crate::web::error::WebError;
use crate::web::invite::validate_invite_code;
use crate::web::state::AppState;

const DEFAULT_SERVER_NAME: &str = "SPT Server";

const FIKA_CLIENT_FORGE_ID: i64 = 2326;
const FIKA_SERVER_FORGE_ID: i64 = 2357;

const BOOTSTRAP_FORGE_IDS: &[i64] = &[
    NARCONET_FORGE_MOD_ID, // 2441
    FIKA_CLIENT_FORGE_ID,  // 2326
    FIKA_SERVER_FORGE_ID,  // 2357
];

#[derive(Debug, serde::Deserialize)]
pub struct JoinQuery {
    pub code: Option<String>,
}

#[derive(Template)]
#[template(path = "join.html")]
struct JoinTemplate {
    server_name: String,
    spt_version: String,
    external_url: String,
    fika_installed: bool,
    modsync_installed: bool,
    mod_count: usize,
    code: String,
    error: Option<String>,
}

fn referrer_policy(resp: HttpResponse) -> HttpResponse {
    let mut resp = resp;
    resp.headers_mut().insert(
        actix_web::http::header::HeaderName::from_static("referrer-policy"),
        actix_web::http::header::HeaderValue::from_static("no-referrer"),
    );
    resp
}

pub async fn join_page(
    query: web::Query<JoinQuery>,
    state: web::Data<AppState>,
) -> actix_web::Result<HttpResponse> {
    let code = query.code.clone().unwrap_or_default();

    // Validate invite code
    let db = state.db.clone();
    let code_clone = code.clone();
    let invite_result = web::block(move || {
        let db = db.lock();
        validate_invite_code(&db, &code_clone)
    })
    .await
    .map_err(WebError::from)?;

    if let Err(e) = invite_result {
        return Ok(referrer_policy(
            HttpResponse::BadRequest()
                .content_type("text/html")
                .body(e.to_string()),
        ));
    }

    let (external_url, server_name) = {
        let config = state.config.read();
        let external_url = match &config.external_url {
            Some(url) => url.clone(),
            None => {
                return Ok(referrer_policy(
                    HttpResponse::ServiceUnavailable()
                        .content_type("text/html")
                        .body(
                            "Bootstrap not configured: external_url is required in quartermaster.toml",
                        ),
                ));
            }
        };

        let server_name = config
            .server_name
            .clone()
            .unwrap_or_else(|| DEFAULT_SERVER_NAME.to_string());
        (external_url, server_name)
    };

    let modsync_installed = state
        .modsync_installed
        .load(std::sync::atomic::Ordering::Relaxed);

    // Count client-syncable mods (mods with BepInEx/ files, excluding infrastructure)
    let db = state.db.clone();
    let mod_count = web::block(move || {
        let db = db.lock();
        db.count_client_syncable_mods()
    })
    .await
    .map_err(WebError::from)?
    .map_err(WebError::from)?;

    let tmpl = JoinTemplate {
        server_name,
        spt_version: state.spt_info.spt_version.clone(),
        external_url,
        fika_installed: state.fika_installed,
        modsync_installed,
        mod_count,
        code,
        error: None,
    };

    Ok(referrer_policy(
        HttpResponse::Ok()
            .content_type("text/html")
            .body(tmpl.render().map_err(WebError::from)?),
    ))
}

pub async fn mod_archive(
    query: web::Query<JoinQuery>,
    state: web::Data<AppState>,
) -> actix_web::Result<HttpResponse> {
    let code = query.code.clone().unwrap_or_default();

    // Validate invite
    let db = state.db.clone();
    let code_clone = code.clone();
    let invite_result = web::block(move || {
        let db = db.lock();
        validate_invite_code(&db, &code_clone)
    })
    .await
    .map_err(WebError::from)?;

    if let Err(e) = invite_result {
        return Ok(referrer_policy(
            HttpResponse::BadRequest()
                .content_type("text/plain")
                .body(e.to_string()),
        ));
    }

    // Get files for bootstrap mods
    let db = state.db.clone();
    let files = web::block(move || {
        let db = db.lock();
        db.get_files_for_forge_ids(BOOTSTRAP_FORGE_IDS)
    })
    .await
    .map_err(WebError::from)?
    .map_err(WebError::from)?;

    if files.is_empty() {
        return Ok(referrer_policy(
            HttpResponse::ServiceUnavailable()
                .content_type("text/plain")
                .body("No bootstrap mods (NarcoNet/Fika) are installed on this server"),
        ));
    }

    // Build ZIP archive in memory
    let spt_dir = state.spt_dir.clone();
    let zip_bytes = web::block(move || build_mod_zip(&spt_dir, &files))
        .await
        .map_err(WebError::from)?
        .map_err(WebError::Internal)?;

    Ok(referrer_policy(
        HttpResponse::Ok()
            .content_type("application/zip")
            .insert_header((
                "content-disposition",
                "attachment; filename=\"quma-mods.zip\"",
            ))
            .body(zip_bytes),
    ))
}

fn build_mod_zip(
    spt_dir: &std::path::Path,
    files: &[crate::db::mods::InstalledFile],
) -> anyhow::Result<Vec<u8>> {
    use std::io::{Cursor, Write};
    use zip::write::SimpleFileOptions;
    use zip::ZipWriter;

    let buf = Cursor::new(Vec::new());
    let mut zip = ZipWriter::new(buf);
    let options = SimpleFileOptions::default().compression_method(zip::CompressionMethod::Deflated);

    for file in files {
        let full_path = spt_dir.join(&file.file_path);
        match std::fs::read(&full_path) {
            Ok(data) => {
                zip.start_file(&file.file_path, options)?;
                zip.write_all(&data)?;
            }
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                tracing::warn!(path = %file.file_path, "skipping missing file in mod archive");
                continue;
            }
            Err(e) => return Err(e.into()),
        }
    }

    let cursor = zip.finish()?;
    Ok(cursor.into_inner())
}
