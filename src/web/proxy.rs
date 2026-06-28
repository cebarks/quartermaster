use std::io::{Read, Write};
use std::time::Instant;

use actix_web::web::{self, Data};
use actix_web::{HttpRequest, HttpResponse};
use flate2::read::ZlibDecoder;
use flate2::write::ZlibEncoder;
use flate2::Compression;
use futures_util::StreamExt;
use reqwest::header::{HeaderMap, HeaderName, HeaderValue};

use crate::web::state::AppState;

enum BackendRewriteTarget {
    HttpProxy,
    DirectTcp,
}

static HOP_BY_HOP_HEADERS: &[&str] = &[
    "connection",
    "keep-alive",
    "proxy-authenticate",
    "proxy-authorization",
    "te",
    "trailers",
    "transfer-encoding",
];

pub async fn proxy_handler(
    req: HttpRequest,
    mut payload: web::Payload,
    state: Data<AppState>,
) -> actix_web::Result<HttpResponse> {
    if !state.config().proxy_enabled {
        return Err(actix_web::error::ErrorNotFound("proxy not enabled"));
    }

    // Detect WebSocket upgrade and delegate to the WS proxy handler
    if req
        .headers()
        .get("upgrade")
        .and_then(|v| v.to_str().ok())
        .map(|v| v.eq_ignore_ascii_case("websocket"))
        .unwrap_or(false)
    {
        return crate::web::proxy_ws::ws_proxy_handler(req, payload, state).await;
    }

    // Read the full body for HTTP requests
    let mut body = web::BytesMut::new();
    while let Some(chunk) = payload.next().await {
        let chunk = chunk.map_err(|e| {
            actix_web::error::ErrorBadRequest(format!("failed to read request body: {e}"))
        })?;
        body.extend_from_slice(&chunk);
    }
    let body = body.freeze();

    let path = req
        .uri()
        .path_and_query()
        .map(|pq| pq.as_str())
        .unwrap_or(req.path());
    let (host, port) = crate::server_detect::resolve_server_addr(&state.config(), &state.spt_dir);
    let upstream_url = format!("https://{host}:{port}{path}");

    let mut headers = HeaderMap::new();
    for (name, value) in req.headers() {
        let name_str = name.as_str().to_lowercase();
        if HOP_BY_HOP_HEADERS.contains(&name_str.as_str()) {
            continue;
        }
        if let Ok(hn) = HeaderName::from_bytes(name.as_str().as_bytes()) {
            if let Ok(hv) = HeaderValue::from_bytes(value.as_bytes()) {
                headers.insert(hn, hv);
            }
        }
    }

    // Convert actix-web Method to reqwest Method
    let method = reqwest::Method::from_bytes(req.method().as_str().as_bytes()).map_err(|e| {
        tracing::error!(err = %e, method = %req.method(), "invalid HTTP method");
        actix_web::error::ErrorBadRequest("invalid HTTP method")
    })?;

    // Clone body for raid tracking BEFORE it's consumed by the upstream request
    let raid_body = if req.method() == actix_web::http::Method::POST
        && (req.path() == "/client/match/local/start" || req.path() == "/client/match/local/end")
    {
        Some(body.clone())
    } else {
        None
    };

    let start = Instant::now();
    let upstream_resp = state
        .proxy_client
        .request(method, &upstream_url)
        .headers(headers)
        .body(body)
        .send()
        .await;

    let latency_ms = start.elapsed().as_millis() as u64;

    match upstream_resp {
        Ok(resp) => {
            let status = resp.status();
            let is_error = status.is_server_error() || status.is_client_error();
            let resp_headers = resp.headers().clone();

            state
                .proxy_metrics
                .record_request(req.path(), latency_ms, is_error);

            // Check if this is a watched endpoint
            let is_register = req.method() == actix_web::http::Method::POST
                && req.path() == "/launcher/profile/register"
                && status.is_success();

            if is_register {
                let spt_dir = state.spt_dir.clone();
                let db = state.db.clone();
                let events = state.events.clone();
                tokio::task::spawn_blocking(move || {
                    handle_player_registration(spt_dir, db, events);
                });
            }

            let is_raid_start = req.method() == actix_web::http::Method::POST
                && req.path() == "/client/match/local/start"
                && status.is_success();

            let is_raid_end = req.method() == actix_web::http::Method::POST
                && req.path() == "/client/match/local/end"
                && status.is_success();

            if is_raid_start || is_raid_end {
                if let Some(profile_id) = crate::web::raid_tracker::extract_session_id(&req) {
                    if let Some(body_clone) = raid_body {
                        let spt_dir = state.spt_dir.clone();
                        let db = state.db.clone();
                        let events = state.events.clone();
                        let snapshots_enabled = state.config().snapshots_enabled;
                        if is_raid_start {
                            tokio::task::spawn_blocking(move || {
                                crate::web::raid_tracker::handle_raid_start(
                                    body_clone,
                                    profile_id,
                                    spt_dir,
                                    db,
                                    events,
                                    snapshots_enabled,
                                );
                            });
                        } else {
                            tokio::task::spawn_blocking(move || {
                                crate::web::raid_tracker::handle_raid_end(
                                    body_clone,
                                    profile_id,
                                    spt_dir,
                                    db,
                                    events,
                                    snapshots_enabled,
                                );
                            });
                        }
                    }
                }
            }

            let client_ip = req
                .peer_addr()
                .map(|a| a.ip().to_string())
                .unwrap_or_default();
            let body_size = resp_headers
                .get("content-length")
                .and_then(|v| v.to_str().ok())
                .and_then(|v| v.parse::<u64>().ok())
                .unwrap_or(0);

            if status.is_server_error() {
                tracing::error!(
                    client_ip = %client_ip,
                    method = %req.method(),
                    path = %req.path(),
                    status = status.as_u16(),
                    latency_ms,
                    body_size,
                    "proxy"
                );
            } else if status.is_client_error() {
                tracing::warn!(
                    client_ip = %client_ip,
                    method = %req.method(),
                    path = %req.path(),
                    status = status.as_u16(),
                    latency_ms,
                    body_size,
                    "proxy"
                );
            } else {
                tracing::debug!(
                    client_ip = %client_ip,
                    method = %req.method(),
                    path = %req.path(),
                    status = status.as_u16(),
                    latency_ms,
                    body_size,
                    "proxy"
                );
            }

            // SPT 4.x hardcodes backend URLs to 127.0.0.1:6969 regardless of
            // config. Rewrite HTTP API URLs to external_url (L7 proxy on :443)
            // and WebSocket/notifier URLs to external_url host on :6969 (direct
            // TCP passthrough) since SPT's notifier breaks under L7 proxies.
            let rewrite_target = if !status.is_success() || state.config().external_url.is_none() {
                None
            } else if req.path() == "/launcher/server/connect"
                || req.path() == "/client/game/config"
            {
                Some(BackendRewriteTarget::HttpProxy)
            } else if req.path() == "/client/notifier/channel/create" {
                Some(BackendRewriteTarget::DirectTcp)
            } else {
                None
            };

            let mut builder = HttpResponse::build(
                actix_web::http::StatusCode::from_u16(status.as_u16())
                    .unwrap_or(actix_web::http::StatusCode::BAD_GATEWAY),
            );

            for (name, value) in resp_headers.iter() {
                let name_str = name.as_str().to_lowercase();
                if HOP_BY_HOP_HEADERS.contains(&name_str.as_str()) {
                    continue;
                }
                if let Ok(value_str) = value.to_str() {
                    builder.insert_header((name.as_str(), value_str));
                }
            }

            if let Some(target) = rewrite_target {
                let external_url = state.config().external_url.clone().unwrap();
                let replacement = match target {
                    BackendRewriteTarget::HttpProxy => extract_host(&external_url),
                    BackendRewriteTarget::DirectTcp => {
                        format!("{}:{}", extract_host(&external_url), port)
                    }
                };
                let raw_body = resp.bytes().await.map_err(|e| {
                    actix_web::error::ErrorBadGateway(format!("failed to read response body: {e}"))
                })?;
                match rewrite_backend_url(&raw_body, &replacement) {
                    Ok(rewritten) => Ok(builder.body(rewritten)),
                    Err(e) => {
                        tracing::warn!(err = %e, "failed to rewrite backend URLs, forwarding original");
                        Ok(builder.body(raw_body))
                    }
                }
            } else {
                let stream = resp.bytes_stream().map(|result| {
                    result.map_err(|e| actix_web::error::PayloadError::Io(std::io::Error::other(e)))
                });
                Ok(builder.streaming(stream))
            }
        }
        Err(e) => {
            state
                .proxy_metrics
                .record_request(req.path(), latency_ms, true);
            tracing::error!(
                method = %req.method(),
                path = %req.path(),
                err = %e,
                latency_ms,
                "proxy upstream unreachable"
            );
            Err(actix_web::error::ErrorBadGateway("SPT server unreachable"))
        }
    }
}

/// Extract the hostname (without scheme or port) from a URL like `https://tarkov.grovest.io`.
fn extract_host(url: &str) -> String {
    url.trim_end_matches('/')
        .strip_prefix("https://")
        .or_else(|| url.strip_prefix("http://"))
        .unwrap_or(url)
        .split(':')
        .next()
        .unwrap_or(url)
        .to_string()
}

/// Rewrite hardcoded `127.0.0.1:6969` in SPT response bodies.
/// `replacement` is the target host or host:port to substitute.
/// The body may be zlib-compressed (SPT default) or plain JSON.
fn rewrite_backend_url(body: &[u8], replacement: &str) -> Result<Vec<u8>, String> {
    let (json_bytes, compressed) = {
        let mut decoder = ZlibDecoder::new(body);
        let mut buf = Vec::new();
        match decoder.read_to_end(&mut buf) {
            Ok(_) => (buf, true),
            Err(_) => (body.to_vec(), false),
        }
    };

    let json_str = String::from_utf8(json_bytes).map_err(|e| format!("utf8: {e}"))?;
    let rewritten = json_str.replace("127.0.0.1:6969", replacement);
    let new_json = rewritten.into_bytes();

    if compressed {
        let mut encoder = ZlibEncoder::new(Vec::new(), Compression::default());
        encoder
            .write_all(&new_json)
            .map_err(|e| format!("zlib compress: {e}"))?;
        encoder.finish().map_err(|e| format!("zlib finish: {e}"))
    } else {
        Ok(new_json)
    }
}

fn handle_player_registration(
    spt_dir: std::path::PathBuf,
    db: std::sync::Arc<parking_lot::Mutex<crate::db::Database>>,
    events: tokio::sync::broadcast::Sender<crate::web::sse::ServerEvent>,
) {
    // SPT's registration endpoint returns an empty 200. To find the new profile,
    // scan the profiles directory for any profile IDs not already in quma's DB.
    //
    // Phase 1: Read all profile data from disk (no DB lock held).
    // Phase 2: Acquire the lock once and perform all DB lookups/inserts.
    let profiles_dir = spt_dir.join("SPT/user/profiles");
    let entries = match std::fs::read_dir(&profiles_dir) {
        Ok(e) => e,
        Err(e) => {
            tracing::warn!(err = %e, "failed to read profiles directory for new user detection");
            return;
        }
    };

    // Phase 1: filesystem I/O — collect profile data without holding the DB lock.
    let mut profiles: Vec<(String, String)> = Vec::new();
    for entry in entries.flatten() {
        let path = entry.path();
        let profile_id = match path.file_stem().and_then(|s| s.to_str()) {
            Some(id) if path.extension().and_then(|e| e.to_str()) == Some("json") => id.to_string(),
            _ => continue,
        };

        // Read profile to get username
        let profile_json: serde_json::Value = match std::fs::read_to_string(&path)
            .ok()
            .and_then(|s| serde_json::from_str(&s).ok())
        {
            Some(v) => v,
            None => continue,
        };

        let username = profile_json
            .pointer("/info/username")
            .and_then(|v| v.as_str())
            .unwrap_or(&profile_id)
            .to_string();

        profiles.push((profile_id, username));
    }

    if profiles.is_empty() {
        return;
    }

    // Phase 2: acquire DB lock once for all lookups and inserts.
    let db = db.lock();
    for (profile_id, username) in &profiles {
        match db.get_user_by_spt_profile_id(profile_id) {
            Ok(Some(_)) => continue,
            Ok(None) => {}
            Err(e) => {
                tracing::warn!(err = %e, profile_id = %profile_id, "failed to check for existing user");
                continue;
            }
        }

        match db.insert_user(username, Some(profile_id), None, "player") {
            Ok(user_id) => {
                tracing::info!(
                    user_id,
                    username = %username,
                    profile_id = %profile_id,
                    "auto-created locked quma account for new SPT player"
                );
                let _ = events.send(crate::web::sse::ServerEvent::PlayerRegistered);
            }
            Err(e) => {
                tracing::warn!(err = %e, username = %username, "failed to auto-create user");
            }
        }
    }
}
