use std::time::Instant;

use actix_web::web::{self, Data};
use actix_web::{HttpRequest, HttpResponse};
use futures_util::StreamExt;
use reqwest::header::{HeaderMap, HeaderName, HeaderValue};

use crate::web::state::AppState;

const PROXY_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(60);

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
    if !state.config.proxy_enabled {
        return Err(actix_web::error::ErrorNotFound("proxy not enabled"));
    }

    // Detect WebSocket upgrade — Task 6 will add the real handler
    if req
        .headers()
        .get("upgrade")
        .and_then(|v| v.to_str().ok())
        .map(|v| v.eq_ignore_ascii_case("websocket"))
        .unwrap_or(false)
    {
        return Err(actix_web::error::ErrorNotImplemented(
            "WebSocket proxy not yet implemented",
        ));
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
    let (host, port) = crate::server_detect::resolve_server_addr(&state.config, &state.spt_dir);
    let upstream_url = format!("https://{host}:{port}{path}");

    let client = build_proxy_client().map_err(|e| {
        tracing::error!(error = %e, "failed to build proxy HTTP client");
        actix_web::error::ErrorBadGateway("proxy client error")
    })?;

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
        tracing::error!(error = %e, method = %req.method(), "invalid HTTP method");
        actix_web::error::ErrorBadRequest("invalid HTTP method")
    })?;

    let start = Instant::now();
    let upstream_resp = client
        .request(method, &upstream_url)
        .headers(headers)
        .body(body.to_vec())
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

            // Check if this is a watched endpoint before consuming the body
            let is_register = req.method() == actix_web::http::Method::POST
                && req.path() == "/launcher/profile/register";

            let response_body = if is_register {
                // Buffer the body for inspection
                match resp.bytes().await {
                    Ok(bytes) => {
                        if status.is_success() {
                            let body_clone = bytes.clone();
                            let db = state.db.clone();
                            let events = state.events.clone();
                            tokio::task::spawn_blocking(move || {
                                handle_player_registration(body_clone, db, events);
                            });
                        }
                        ProxyBody::Buffered(bytes)
                    }
                    Err(e) => {
                        tracing::warn!(error = %e, "failed to read registration response body");
                        ProxyBody::Empty
                    }
                }
            } else {
                ProxyBody::Stream(resp)
            };

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
                tracing::info!(
                    client_ip = %client_ip,
                    method = %req.method(),
                    path = %req.path(),
                    status = status.as_u16(),
                    latency_ms,
                    body_size,
                    "proxy"
                );
            }

            let mut builder = HttpResponse::build(
                actix_web::http::StatusCode::from_u16(status.as_u16())
                    .unwrap_or(actix_web::http::StatusCode::BAD_GATEWAY),
            );

            for (name, value) in resp_headers.iter() {
                let name_str = name.as_str().to_lowercase();
                if HOP_BY_HOP_HEADERS.contains(&name_str.as_str()) {
                    continue;
                }
                // Convert reqwest header types to actix-web compatible types
                if let Ok(value_str) = value.to_str() {
                    builder.insert_header((name.as_str(), value_str));
                }
            }

            match response_body {
                ProxyBody::Buffered(bytes) => Ok(builder.body(bytes)),
                ProxyBody::Stream(resp) => {
                    let stream = resp.bytes_stream().map(|result| {
                        result
                            .map(|bytes| web::Bytes::from(bytes.to_vec()))
                            .map_err(|e| {
                                actix_web::error::PayloadError::Io(std::io::Error::other(e))
                            })
                    });
                    Ok(builder.streaming(stream))
                }
                ProxyBody::Empty => Ok(builder.finish()),
            }
        }
        Err(e) => {
            state
                .proxy_metrics
                .record_request(req.path(), latency_ms, true);
            tracing::error!(
                method = %req.method(),
                path = %req.path(),
                error = %e,
                latency_ms,
                "proxy upstream unreachable"
            );
            Err(actix_web::error::ErrorBadGateway("SPT server unreachable"))
        }
    }
}

enum ProxyBody {
    Buffered(web::Bytes),
    Stream(reqwest::Response),
    Empty,
}

fn build_proxy_client() -> Result<reqwest::Client, reqwest::Error> {
    reqwest::Client::builder()
        .danger_accept_invalid_certs(true)
        .timeout(PROXY_TIMEOUT)
        .redirect(reqwest::redirect::Policy::none())
        .build()
}

fn handle_player_registration(
    body: web::Bytes,
    db: std::sync::Arc<parking_lot::Mutex<crate::db::Database>>,
    events: tokio::sync::broadcast::Sender<crate::web::sse::ServerEvent>,
) {
    let body_str = match std::str::from_utf8(&body) {
        Ok(s) => s,
        Err(e) => {
            tracing::warn!(error = %e, "registration response is not valid UTF-8");
            return;
        }
    };

    // Try to parse as JSON — the exact format will be discovered during testing.
    // Common SPT patterns: the response may be a JSON string (profile ID) or a JSON object.
    let json: serde_json::Value = match serde_json::from_str(body_str) {
        Ok(v) => v,
        Err(e) => {
            tracing::warn!(error = %e, body = %body_str, "failed to parse registration response as JSON");
            return;
        }
    };

    // Extract profile_id — try common field names
    let profile_id = json
        .get("profileId")
        .or_else(|| json.get("profile_id"))
        .or_else(|| json.get("id"))
        .and_then(|v| v.as_str())
        .or_else(|| {
            // SPT may return the profile ID as a bare JSON string
            json.as_str()
        });

    let profile_id = match profile_id {
        Some(id) if !id.is_empty() => id.to_string(),
        _ => {
            tracing::warn!(body = %body_str, "could not extract profile ID from registration response");
            return;
        }
    };

    // Extract username if available
    let username = json
        .get("username")
        .or_else(|| json.get("nickname"))
        .and_then(|v| v.as_str())
        .unwrap_or(&profile_id);

    let db = db.lock();

    match db.get_user_by_spt_profile_id(&profile_id) {
        Ok(Some(_)) => {
            tracing::info!(profile_id = %profile_id, "player already exists in quma, skipping auto-create");
            return;
        }
        Ok(None) => {}
        Err(e) => {
            tracing::warn!(error = %e, profile_id = %profile_id, "failed to check for existing user");
            return;
        }
    }

    match db.insert_user(
        username,
        Some(&profile_id),
        None,
        crate::db::users::Role::Player,
    ) {
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
            tracing::warn!(error = %e, username = %username, "failed to auto-create user");
        }
    }
}
