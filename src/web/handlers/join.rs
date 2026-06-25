use actix_web::{web, HttpResponse};
use askama::Template;

use crate::web::error::WebError;
use crate::web::invite::validate_invite_code;
use crate::web::state::AppState;

const DEFAULT_SERVER_NAME: &str = "SPT Server";

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
