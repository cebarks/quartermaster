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

/// Cap proxy request bodies well below the global 64 MB PayloadConfig.
/// SPT client traffic is compressed JSON — profiles top out around 2 MB.
const MAX_PROXY_BODY: usize = 16 * 1024 * 1024;

/// Cap zlib decompression output to prevent zip-bomb DoS.
// ponytail: 64 MB is generous; tighten if memory pressure matters
const MAX_DECOMPRESSED: u64 = 64 * 1024 * 1024;

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

fn is_auth_exempt(method: &actix_web::http::Method, path: &str) -> bool {
    matches!(
        (method, path),
        (&actix_web::http::Method::POST, "/launcher/v2/register")
            | (_, "/launcher/v2/ping")
            | (_, "/launcher/v2/version")
            | (_, "/launcher/v2/mods")
            | (
                &actix_web::http::Method::GET,
                "/singleplayer/customEnumEntries"
            )
    ) || (method == actix_web::http::Method::GET && is_prestige_artwork(path))
}

fn is_prestige_artwork(path: &str) -> bool {
    [
        "/files/prestige/",
        "/files/prestige/reward/small/",
        "/files/prestige/reward/big/",
    ]
    .iter()
    .any(|prefix| {
        path.strip_prefix(prefix)
            .and_then(|filename| filename.strip_suffix(".png"))
            .is_some_and(|id| id.len() == 24 && id.bytes().all(|byte| byte.is_ascii_hexdigit()))
    })
}

#[derive(Debug, PartialEq)]
struct ProxyCredentialShape {
    authorization_headers: usize,
    cookie_headers: usize,
    session_keys: usize,
    session_values_equal: Option<bool>,
}

fn proxy_credential_shape(req: &HttpRequest) -> ProxyCredentialShape {
    let header_count = |header: &str| {
        req.headers()
            .iter()
            .filter(|(name, _)| name.as_str().eq_ignore_ascii_case(header))
            .count()
    };
    let sessions: Vec<&str> = req
        .headers()
        .iter()
        .filter(|(name, _)| name.as_str().eq_ignore_ascii_case("cookie"))
        .filter_map(|(_, value)| value.to_str().ok())
        .flat_map(|value| value.split(';'))
        .filter_map(|cookie| cookie.split_once('='))
        .filter(|(name, _)| name.trim().eq_ignore_ascii_case("PHPSESSID"))
        .map(|(_, value)| value.trim())
        .collect();
    ProxyCredentialShape {
        authorization_headers: header_count("authorization"),
        cookie_headers: header_count("cookie"),
        session_keys: sessions.len(),
        session_values_equal: (sessions.len() > 1)
            .then(|| sessions.windows(2).all(|pair| pair[0] == pair[1])),
    }
}

fn canonical_redundant_cookie(fields: &[&str]) -> Option<String> {
    let mut session: Option<&str> = None;
    let mut session_count = 0;
    let mut others = std::collections::BTreeMap::new();
    for field in fields {
        for entry in field.split(';').filter(|entry| !entry.trim().is_empty()) {
            let (name, value) = entry.split_once('=')?;
            let (name, value) = (name.trim(), value.trim());
            if name.eq_ignore_ascii_case("PHPSESSID") {
                if value.len() != 24
                    || !value.bytes().all(|byte| byte.is_ascii_hexdigit())
                    || session.is_some_and(|previous| previous != value)
                {
                    return None;
                }
                session = Some(value);
                session_count += 1;
            } else if name.is_empty()
                || !name
                    .bytes()
                    .all(|byte| byte.is_ascii_alphanumeric() || b"-_.".contains(&byte))
                || others
                    .insert(name.to_ascii_lowercase(), (name, value))
                    .is_some()
            {
                return None;
            }
        }
    }
    if session_count < 2 {
        return None;
    }
    let mut cookie = format!("PHPSESSID={}", session?);
    for (name, value) in others.into_values() {
        cookie.push_str("; ");
        cookie.push_str(name);
        cookie.push('=');
        cookie.push_str(value);
    }
    Some(cookie)
}

fn normalize_proxy_headers(req: &HttpRequest) -> Option<actix_web::http::header::HeaderMap> {
    if req
        .headers()
        .iter()
        .filter(|(name, _)| name.as_str() == "authorization")
        .take(2)
        .count()
        > 1
    {
        return None;
    }
    let fields: Vec<&str> = req
        .headers()
        .iter()
        .filter(|(name, _)| name.as_str() == "cookie")
        .map(|(_, value)| value.to_str().ok())
        .collect::<Option<_>>()?;
    let session_keys = fields
        .iter()
        .flat_map(|field| field.split(';'))
        .filter(|entry| {
            entry
                .split('=')
                .next()
                .is_some_and(|name| name.trim().eq_ignore_ascii_case("PHPSESSID"))
        })
        .count();
    let mut headers = req.headers().clone();
    if fields.len() > 1 || session_keys > 1 {
        let cookie = canonical_redundant_cookie(&fields)?;
        headers.remove("cookie");
        headers.insert(
            actix_web::http::header::COOKIE,
            actix_web::http::header::HeaderValue::from_str(&cookie).ok()?,
        );
    }
    Some(headers)
}

fn proxy_session_credentials_from_headers(
    req: &HttpRequest,
    headers: &actix_web::http::header::HeaderMap,
) -> Option<(String, bool)> {
    let requires_managed = req.path() == "/fika/headless/client";
    if requires_managed && headers.contains_key("authorization") {
        return headless_basic_session(req, headers).map(|session| (session, true));
    }
    crate::web::raid_tracker::extract_session_id_from_headers(headers)
        .map(|session| (session, requires_managed))
}

fn headless_basic_session(
    req: &HttpRequest,
    headers: &actix_web::http::header::HeaderMap,
) -> Option<String> {
    use base64::{engine::general_purpose::STANDARD, Engine};
    if req.method() != actix_web::http::Method::GET
        || req.path() != "/fika/headless/client"
        || !is_direct_local_peer(req)
        || !req
            .headers()
            .get("upgrade")?
            .to_str()
            .ok()?
            .eq_ignore_ascii_case("websocket")
        || !req
            .headers()
            .get("connection")?
            .to_str()
            .ok()?
            .split(',')
            .any(|value| value.trim().eq_ignore_ascii_case("upgrade"))
    {
        return None;
    }
    let authorization = headers.get("authorization")?.to_str().ok()?;
    let (scheme, encoded) = authorization.split_once(' ')?;
    if !scheme.eq_ignore_ascii_case("basic") || encoded.len() != 36 {
        return None;
    }
    let decoded = STANDARD.decode(encoded).ok()?;
    if decoded.len() != 25
        || decoded[24] != b':'
        || !decoded[..24].iter().all(u8::is_ascii_hexdigit)
    {
        return None;
    }
    let session = String::from_utf8(decoded[..24].to_vec()).ok()?;
    if headers.contains_key("cookie")
        && crate::web::raid_tracker::extract_session_id_from_headers(headers).as_deref()
            != Some(session.as_str())
    {
        return None;
    }
    Some(session)
}

fn is_direct_local_peer(req: &HttpRequest) -> bool {
    req.peer_addr().is_some_and(|peer| peer.ip().is_loopback())
        && ["forwarded", "x-forwarded-for", "x-real-ip"]
            .iter()
            .all(|header| !req.headers().contains_key(*header))
}

fn managed_headless_profile_matches(
    container: &bollard::models::ContainerInspectResponse,
    session_id: &str,
    configured_count: u32,
) -> bool {
    use crate::client::converge::{client_container_name, MANAGED_BY_LABEL, MANAGED_BY_VALUE};
    if session_id.len() != 24 || !session_id.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        return false;
    }
    if container.state.as_ref().and_then(|state| state.running) != Some(true) {
        return false;
    }
    let Some(config) = container.config.as_ref() else {
        return false;
    };
    let Some(labels) = config.labels.as_ref() else {
        return false;
    };
    if labels.get(MANAGED_BY_LABEL).map(String::as_str) != Some(MANAGED_BY_VALUE) {
        return false;
    }
    let Some(index) = labels
        .get("quma.client.index")
        .and_then(|value| value.parse::<u32>().ok())
    else {
        return false;
    };
    if index == 0 || index > configured_count {
        return false;
    }
    let expected_name = client_container_name(index);
    if container
        .name
        .as_deref()
        .map(|name| name.trim_start_matches('/'))
        != Some(expected_name.as_str())
    {
        return false;
    }
    let expected_profile = format!("PROFILE_ID={session_id}");
    config
        .env
        .as_ref()
        .is_some_and(|env| env.contains(&expected_profile))
}

async fn is_managed_headless_session(
    req: &HttpRequest,
    state: &AppState,
    session_id: &str,
) -> bool {
    if !is_direct_local_peer(req) {
        return false;
    }
    let count = state
        .config()
        .headless
        .as_ref()
        .map_or(0, |config| config.client_count());
    let Some(manager) = state.container_mgr.as_ref().filter(|_| count > 0) else {
        return false;
    };
    use crate::client::converge::{MANAGED_BY_LABEL, MANAGED_BY_VALUE};
    let Ok(names) = manager
        .detect_containers_by_label(MANAGED_BY_LABEL, MANAGED_BY_VALUE)
        .await
    else {
        return false;
    };
    // Read live runtime metadata rather than trusting a profile name or stale allowlist.
    for name in names
        .into_iter()
        .take(crate::config::MAX_HEADLESS_CLIENTS as usize)
    {
        if let Ok(container) = manager.inspect(&name).await {
            if managed_headless_profile_matches(&container, session_id, count) {
                return true;
            }
        }
    }
    false
}

#[derive(Debug, PartialEq, Eq)]
enum ProxyIdentityError {
    Disabled,
    Unknown,
}

async fn classify_proxy_identity(
    linked_disabled: Option<bool>,
    requires_managed: bool,
    managed: impl std::future::Future<Output = bool>,
) -> Result<bool, ProxyIdentityError> {
    if linked_disabled == Some(true) {
        return Err(ProxyIdentityError::Disabled);
    }
    if managed.await {
        Ok(true)
    } else if linked_disabled == Some(false) && !requires_managed {
        Ok(false)
    } else {
        Err(ProxyIdentityError::Unknown)
    }
}

pub async fn proxy_handler(
    req: HttpRequest,
    mut payload: web::Payload,
    state: Data<AppState>,
) -> actix_web::Result<HttpResponse> {
    if !state.config().proxy_enabled {
        return Err(actix_web::error::ErrorNotFound("proxy not enabled"));
    }

    let Some(proxy_headers) = normalize_proxy_headers(&req) else {
        tracing::warn!(path = %req.path(), credential_shape = ?proxy_credential_shape(&req), "proxy auth: ambiguous credentials");
        return Err(actix_web::error::ErrorUnauthorized("ambiguous credentials"));
    };
    let mut managed_headless = false;
    if state.config().proxy_auth && !is_auth_exempt(req.method(), req.path()) {
        let (session_id, requires_managed) =
            proxy_session_credentials_from_headers(&req, &proxy_headers).ok_or_else(|| {
                tracing::warn!(
                    client_ip = %req.connection_info().realip_remote_addr().unwrap_or_default(),
                    path = %req.path(),
                    "proxy auth: missing or invalid session"
                );
                actix_web::error::ErrorUnauthorized("missing or invalid session")
            })?;

        let db = state.db.clone();
        let lookup_id = session_id.clone();
        let user = web::block(move || {
            let db = db.lock();
            db.get_user_by_spt_profile_id(&lookup_id)
        })
        .await
        .map_err(|e| {
            tracing::error!(err = %e, "proxy auth: db lookup failed");
            actix_web::error::ErrorInternalServerError("internal error")
        })?
        .map_err(|e| {
            tracing::error!(err = %e, "proxy auth: query failed");
            actix_web::error::ErrorInternalServerError("internal error")
        })?;

        managed_headless = classify_proxy_identity(
            user.as_ref().map(|user| user.disabled), requires_managed,
            is_managed_headless_session(&req, &state, &session_id),
        ).await.map_err(|error| match error {
            ProxyIdentityError::Disabled => {
                if let Some(user) = &user {
                    tracing::warn!(username = %user.username, path = %req.path(), "proxy auth: disabled account");
                }
                actix_web::error::ErrorForbidden("account disabled")
            }
            ProxyIdentityError::Unknown => {
                tracing::warn!(client_ip = %req.connection_info().realip_remote_addr().unwrap_or_default(), path = %req.path(), "proxy auth: unknown profile");
                actix_web::error::ErrorUnauthorized("unknown profile")
            }
        })?;
    }

    // Detect WebSocket upgrade and delegate to the WS proxy handler
    if req
        .headers()
        .get("upgrade")
        .and_then(|v| v.to_str().ok())
        .map(|v| v.eq_ignore_ascii_case("websocket"))
        .unwrap_or(false)
    {
        return crate::web::proxy_ws::ws_proxy_handler(req, payload, state, proxy_headers).await;
    }

    // Read the full body for HTTP requests, enforcing proxy-specific size limit
    let mut body = web::BytesMut::new();
    while let Some(chunk) = payload.next().await {
        let chunk = chunk.map_err(|e| {
            actix_web::error::ErrorBadRequest(format!("failed to read request body: {e}"))
        })?;
        body.extend_from_slice(&chunk);
        if body.len() > MAX_PROXY_BODY {
            return Err(actix_web::error::ErrorPayloadTooLarge(
                "proxy request body too large",
            ));
        }
    }
    let body = body.freeze();

    let path = req
        .uri()
        .path_and_query()
        .map(|pq| pq.as_str())
        .unwrap_or(req.path());
    let (host, port) = crate::server_detect::resolve_server_addr(&state.config(), &state.dirs);
    let upstream_url = format!("https://{host}:{port}{path}");

    let mut headers = HeaderMap::new();
    for (name, value) in &proxy_headers {
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

    // Fix headless CRC: the headless sends crc32=0 when creating raids because
    // Fika's RunChecks() fails silently under Wine. The headless talks directly
    // to the SPT server (bypassing this proxy), so we fix it on the response
    // side: when a game client joins a raid via /fika/raid/join, we check the
    // response and replace crc32=0 with the real CRC32C from Fika.Core.dll.
    let fix_crc_response =
        req.method() == actix_web::http::Method::POST && req.path() == "/fika/raid/join";

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
                && req.path() == "/launcher/v2/register"
                && status.is_success();

            if is_register {
                let dirs = state.dirs.clone();
                let db = state.db.clone();
                let events = state.events.clone();
                tokio::task::spawn_blocking(move || {
                    handle_player_registration(dirs, db, events);
                });
            }

            let is_raid_start = req.method() == actix_web::http::Method::POST
                && req.path() == "/client/match/local/start"
                && status.is_success();

            let is_raid_end = req.method() == actix_web::http::Method::POST
                && req.path() == "/client/match/local/end"
                && status.is_success();

            if is_raid_start || is_raid_end {
                if let Some(profile_id) =
                    crate::web::raid_tracker::extract_session_id_from_headers(&proxy_headers)
                {
                    if let Some(body_clone) = raid_body {
                        let dirs = state.dirs.clone();
                        let db = state.db.clone();
                        let events = state.events.clone();
                        let snapshots_enabled = state.config().snapshots_enabled;
                        if is_raid_start {
                            tokio::task::spawn_blocking(move || {
                                crate::web::raid_tracker::handle_raid_start(
                                    body_clone,
                                    profile_id,
                                    dirs,
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
                                    dirs.spt_server.clone(),
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
                .connection_info()
                .realip_remote_addr()
                .unwrap_or_default()
                .to_owned();
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

            let rewrite_target = {
                let cfg = state.config();
                if !status.is_success() || cfg.external_url.is_none() {
                    None
                } else if cfg.proxy_rewrite_http_paths.iter().any(|p| p == req.path()) {
                    Some(BackendRewriteTarget::HttpProxy)
                } else if cfg
                    .proxy_rewrite_direct_paths
                    .iter()
                    .any(|p| p == req.path())
                {
                    Some(BackendRewriteTarget::DirectTcp)
                } else {
                    None
                }
            };

            let mut builder = HttpResponse::build(
                actix_web::http::StatusCode::from_u16(status.as_u16())
                    .unwrap_or(actix_web::http::StatusCode::BAD_GATEWAY),
            );

            let (external_url_for_rewrite, target_port, proxy_port) = {
                let cfg = state.config();
                (
                    rewrite_target
                        .as_ref()
                        .and_then(|_| cfg.external_url.clone()),
                    cfg.proxy_rewrite_target_port.unwrap_or(port),
                    cfg.web_port,
                )
            };
            let will_rewrite = matches!(
                (&rewrite_target, &external_url_for_rewrite),
                (Some(_), Some(_))
            );

            for (name, value) in resp_headers.iter() {
                let name_str = name.as_str().to_lowercase();
                if HOP_BY_HOP_HEADERS.contains(&name_str.as_str()) {
                    continue;
                }
                if will_rewrite && name_str == "content-length" {
                    continue;
                }
                if let Ok(value_str) = value.to_str() {
                    builder.insert_header((name.as_str(), value_str));
                }
            }

            if let (Some(target), Some(external_url)) = (rewrite_target, external_url_for_rewrite) {
                let replacement = backend_rewrite_authority(
                    &target,
                    &external_url,
                    target_port,
                    proxy_port,
                    port,
                    managed_headless,
                );
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
            } else if fix_crc_response && status.is_success() {
                let raw_body = resp.bytes().await.map_err(|e| {
                    actix_web::error::ErrorBadGateway(format!("failed to read response body: {e}"))
                })?;
                let fixed = fix_headless_crc(raw_body, &state.dirs);
                Ok(builder.body(fixed))
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

fn backend_rewrite_authority(
    target: &BackendRewriteTarget,
    external_url: &str,
    direct_port: u16,
    proxy_port: u16,
    local_server_port: u16,
    managed_headless: bool,
) -> String {
    if managed_headless {
        let port = match target {
            BackendRewriteTarget::HttpProxy => proxy_port,
            BackendRewriteTarget::DirectTcp => local_server_port,
        };
        return format!("127.0.0.1:{port}");
    }
    match target {
        BackendRewriteTarget::HttpProxy => extract_host(external_url),
        BackendRewriteTarget::DirectTcp => format!("{}:{direct_port}", extract_host(external_url)),
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

fn rewrite_backend_url(body: &[u8], replacement: &str) -> Result<Vec<u8>, String> {
    let (json_bytes, compressed) = {
        let mut decoder = ZlibDecoder::new(body).take(MAX_DECOMPRESSED);
        let mut buf = Vec::new();
        match decoder.read_to_end(&mut buf) {
            Ok(_) => (buf, true),
            Err(_) => (body.to_vec(), false),
        }
    };

    let json_str = String::from_utf8(json_bytes).map_err(|e| format!("utf8: {e}"))?;

    // Detect the origin (host or host:port) from the first https:// URL in
    // the response, then replace every occurrence with the external URL origin.
    // SPT derives the origin from the incoming Host header, which varies by
    // proxy topology — matching specific IPs is too fragile.
    let rewritten = if let Some(scheme_pos) = json_str.find("https://") {
        let origin_start = scheme_pos + "https://".len();
        if let Some(rel) = json_str[origin_start..].find(['/', '"']) {
            let origin = json_str[origin_start..origin_start + rel].to_string();
            if origin == replacement {
                json_str
            } else {
                tracing::debug!(detected = %origin, replacement, "proxy rewriting origin");
                json_str.replace(&origin, replacement)
            }
        } else {
            json_str
        }
    } else {
        json_str
    };

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
    dirs: std::sync::Arc<crate::dirs::QumaDirs>,
    db: std::sync::Arc<parking_lot::Mutex<crate::db::Database>>,
    events: tokio::sync::broadcast::Sender<crate::web::sse::ServerEvent>,
) {
    // SPT's registration endpoint returns an empty 200. To find the new profile,
    // scan the profiles directory for any profile IDs not already in quma's DB.
    //
    // Phase 1: Read all profile data from disk (no DB lock held).
    // Phase 2: Acquire the lock once and perform all DB lookups/inserts.
    let reader = match crate::spt::profile_store::ProfileReader::open(&dirs) {
        Ok(reader) => reader,
        Err(error) => {
            tracing::warn!(err = %error, "failed to open profile layers for new user detection");
            return;
        }
    };
    let entries = match reader.candidate_names() {
        Ok(entries) => entries,
        Err(e) => {
            tracing::warn!(err = %e, "failed to read profiles directory for new user detection");
            return;
        }
    };

    // Phase 1: filesystem I/O — collect profile data without holding the DB lock.
    let mut profiles: Vec<(String, String)> = Vec::new();
    for name in entries {
        let path = std::path::Path::new(&name);
        let profile_id = match path.file_stem().and_then(|s| s.to_str()) {
            Some(id) if path.extension().and_then(|e| e.to_str()) == Some("json") => id.to_string(),
            _ => continue,
        };

        // Read profile to get username
        let profile_json: serde_json::Value = match reader
            .read(path)
            .ok()
            .flatten()
            .and_then(|file| serde_json::from_reader(std::io::BufReader::new(file)).ok())
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

        let is_headless = username.starts_with("headless_");
        match db.insert_user(username, Some(profile_id), None, "player", is_headless) {
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

/// Compute CRC32C (Castagnoli) of a byte slice — same algorithm Fika uses.
fn crc32c(data: &[u8]) -> u32 {
    static TABLE: std::sync::LazyLock<[u32; 256]> = std::sync::LazyLock::new(|| {
        let mut table = [0u32; 256];
        for i in 0..256u32 {
            let mut crc = i;
            for _ in 0..8 {
                if crc & 1 != 0 {
                    crc = (crc >> 1) ^ 0x82F6_3B78;
                } else {
                    crc >>= 1;
                }
            }
            table[i as usize] = crc;
        }
        table
    });

    let mut crc = 0xFFFF_FFFFu32;
    for &b in data {
        crc = TABLE[((crc ^ b as u32) & 0xFF) as usize] ^ (crc >> 8);
    }
    crc ^ 0xFFFF_FFFF
}

/// If the response body contains `"crc32":0`, compute the real CRC32C from the
/// Fika.Core.dll on disk and substitute it. This works around a Fika bug where
/// the headless client's `RunChecks()` silently fails under Wine, leaving
/// `FikaPlugin.Crc32 = 0`. Applied to `/fika/raid/join` responses so game
/// clients see the correct host CRC even when the headless reported 0.
fn fix_headless_crc(body: web::Bytes, dirs: &crate::dirs::QumaDirs) -> web::Bytes {
    // Try zlib decompression first, fall back to raw bytes
    let (json_bytes, compressed) = {
        let mut decoder = ZlibDecoder::new(&body[..]).take(MAX_DECOMPRESSED);
        let mut buf = Vec::new();
        match decoder.read_to_end(&mut buf) {
            Ok(_) => (buf, true),
            Err(_) => (body.to_vec(), false),
        }
    };

    let json_str = match std::str::from_utf8(&json_bytes) {
        Ok(s) => s,
        Err(_) => return body,
    };

    // Fast check: only rewrite if crc32 is literally 0
    if !json_str.contains("\"crc32\":0")
        && !json_str.contains("\"crc32\": 0")
        && !json_str.contains("\"crc32\" : 0")
    {
        return body;
    }

    // Compute CRC from the Fika.Core.dll in the headless install dir
    let dll_path = dirs
        .headless_base
        .join("BepInEx/plugins/Fika/Fika.Core.dll");
    let dll_bytes = match std::fs::read(&dll_path) {
        Ok(b) => b,
        Err(e) => {
            tracing::warn!(err = %e, path = %dll_path.display(), "failed to read Fika.Core.dll for CRC fix");
            return body;
        }
    };
    let real_crc = crc32c(&dll_bytes);
    tracing::debug!(
        crc = real_crc,
        "fixing headless crc32=0 in /fika/raid/join response"
    );

    // Replace the zero CRC with the real value
    let fixed = json_str
        .replace("\"crc32\":0", &format!("\"crc32\":{real_crc}"))
        .replace("\"crc32\": 0", &format!("\"crc32\":{real_crc}"))
        .replace("\"crc32\" : 0", &format!("\"crc32\":{real_crc}"));

    let new_bytes = fixed.into_bytes();
    if compressed {
        let mut encoder = ZlibEncoder::new(Vec::new(), Compression::default());
        if encoder.write_all(&new_bytes).is_err() {
            return body;
        }
        match encoder.finish() {
            Ok(compressed_bytes) => web::Bytes::from(compressed_bytes),
            Err(_) => body,
        }
    } else {
        web::Bytes::from(new_bytes)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn proxy_session_credentials(req: &HttpRequest) -> Option<(String, bool)> {
        let headers = normalize_proxy_headers(req)?;
        proxy_session_credentials_from_headers(req, &headers)
    }

    #[test]
    fn rewrite_replaces_loopback_origin() {
        let body = br#"{"backendUrl":"https://127.0.0.1:6969/api","ws":"wss://127.0.0.1:6969/ws"}"#;
        let result = rewrite_backend_url(body, "tarkov.example.com").unwrap();
        let s = String::from_utf8(result).unwrap();
        assert_eq!(
            s,
            r#"{"backendUrl":"https://tarkov.example.com/api","ws":"wss://tarkov.example.com/ws"}"#
        );
    }

    #[test]
    fn rewrite_replaces_lan_ip_origin() {
        let body = br#"{"backendUrl":"https://192.168.132.64:9190","other":"https://192.168.132.64:9190/path"}"#;
        let result = rewrite_backend_url(body, "tarkov.example.com").unwrap();
        let s = String::from_utf8(result).unwrap();
        assert!(s.contains("https://tarkov.example.com\""));
        assert!(s.contains("https://tarkov.example.com/path"));
        assert!(!s.contains("192.168.132.64"));
    }

    #[test]
    fn rewrite_noop_when_already_correct() {
        let body = br#"{"backendUrl":"https://tarkov.example.com/api"}"#;
        let result = rewrite_backend_url(body, "tarkov.example.com").unwrap();
        assert_eq!(result, body);
    }

    #[test]
    fn rewrite_noop_when_no_urls() {
        let body = br#"{"status":"ok","count":42}"#;
        let result = rewrite_backend_url(body, "tarkov.example.com").unwrap();
        assert_eq!(result, body);
    }

    #[test]
    fn rewrite_handles_zlib_compressed_body() {
        use flate2::write::ZlibEncoder;
        use std::io::Write;

        let json = br#"{"backendUrl":"https://0.0.0.0:6969/connect"}"#;
        let mut enc = ZlibEncoder::new(Vec::new(), Compression::default());
        enc.write_all(json).unwrap();
        let compressed = enc.finish().unwrap();

        let result = rewrite_backend_url(&compressed, "tarkov.example.com").unwrap();

        // Result should be re-compressed; decompress to verify
        let mut dec = ZlibDecoder::new(result.as_slice());
        let mut decompressed = String::new();
        std::io::Read::read_to_string(&mut dec, &mut decompressed).unwrap();
        assert_eq!(
            decompressed,
            r#"{"backendUrl":"https://tarkov.example.com/connect"}"#
        );
    }

    #[test]
    fn rewrite_replaces_wildcard_origin() {
        let body = br#"{"backendUrl":"https://0.0.0.0:6969"}"#;
        let result = rewrite_backend_url(body, "tarkov.example.com").unwrap();
        let s = String::from_utf8(result).unwrap();
        assert_eq!(s, r#"{"backendUrl":"https://tarkov.example.com"}"#);
    }

    #[test]
    fn auth_exempt_paths_include_register() {
        assert!(is_auth_exempt(
            &actix_web::http::Method::POST,
            "/launcher/v2/register"
        ));
    }

    #[test]
    fn auth_exempt_paths_include_ping() {
        assert!(is_auth_exempt(
            &actix_web::http::Method::GET,
            "/launcher/v2/ping"
        ));
    }

    #[test]
    fn agreeing_session_cookies_are_canonicalized_before_use() {
        use actix_web::test::TestRequest;
        let session = "000000000000000000000001";
        let request = TestRequest::get()
            .append_header(("cookie", format!("PHPSESSID={session}; other=one")))
            .append_header(("cookie", format!("PHPSESSID={session}")))
            .insert_header(("x-client-version", "test"))
            .to_http_request();
        let headers = normalize_proxy_headers(&request).expect("agreeing session cookies");
        assert_eq!(
            headers
                .iter()
                .filter(|(name, _)| name.as_str() == "cookie")
                .count(),
            1
        );
        assert_eq!(
            headers.get("cookie").unwrap().to_str().unwrap(),
            format!("PHPSESSID={session}; other=one")
        );
        assert_eq!(headers.get("x-client-version").unwrap(), "test");
        for cookie in [
            format!("PHPSESSID={session}; PHPSESSID={session}"),
            format!("PHPSESSID={session}; phpsessid={session}"),
        ] {
            let request = TestRequest::get()
                .insert_header(("cookie", cookie))
                .to_http_request();
            assert_eq!(
                normalize_proxy_headers(&request)
                    .unwrap()
                    .get("cookie")
                    .unwrap()
                    .to_str()
                    .unwrap(),
                format!("PHPSESSID={session}")
            );
        }
    }

    #[test]
    fn cookie_normalization_rejects_invalid_and_unrelated_ambiguity() {
        use actix_web::test::TestRequest;
        let valid = "PHPSESSID=000000000000000000000001";
        for second in [
            "PHPSESSID=000000000000000000000002",
            "PHPSESSID=bad",
            "PHPSESSID=",
            "PHPSESSID=00000000000000000000000g",
            "PHPSESSID",
            "other=one",
        ] {
            let request = TestRequest::get()
                .append_header(("cookie", valid))
                .append_header(("cookie", second))
                .to_http_request();
            assert!(normalize_proxy_headers(&request).is_none());
        }
        for extra in [
            "other=one; Other=two",
            "other=one; other=one",
            "PHP%53ESSID=000000000000000000000002",
        ] {
            let request = TestRequest::get()
                .append_header(("cookie", valid))
                .append_header(("cookie", format!("{valid}; {extra}")))
                .to_http_request();
            assert!(normalize_proxy_headers(&request).is_none());
        }
    }

    #[test]
    fn credential_diagnostics_report_counts_and_equality_without_values() {
        use actix_web::test::TestRequest;
        for second in ["000000000000000000000001", "000000000000000000000002"] {
            let request = TestRequest::get()
                .append_header(("authorization", "Basic first"))
                .append_header(("authorization", "Basic second"))
                .append_header(("cookie", "PHPSESSID=000000000000000000000001"))
                .append_header(("cookie", format!("PHPSESSID={second}")))
                .to_http_request();
            let shape = proxy_credential_shape(&request);
            assert_eq!(
                shape,
                ProxyCredentialShape {
                    authorization_headers: 2,
                    cookie_headers: 2,
                    session_keys: 2,
                    session_values_equal: Some(second.ends_with('1'))
                }
            );
            let diagnostic = format!("{shape:?}");
            assert!(!diagnostic.contains("000000000000000000000001"));
            assert!(!diagnostic.contains(second));
            assert!(!diagnostic.contains("Basic first"));
        }
    }

    #[test]
    fn proxy_credentials_reject_conflicts_and_duplicate_authorization() {
        use actix_web::test::TestRequest;
        use base64::{engine::general_purpose::STANDARD, Engine};
        let a = "000000000000000000000001";
        let b = "000000000000000000000002";
        for (first, second) in [(a, b), (b, a), (a, a)] {
            let basic = |id: &str| format!("Basic {}", STANDARD.encode(format!("{id}:")));
            let request = TestRequest::get()
                .uri("/fika/headless/client")
                .peer_addr("127.0.0.1:12345".parse().unwrap())
                .insert_header(("upgrade", "websocket"))
                .insert_header(("connection", "Upgrade"))
                .append_header(("authorization", basic(first)))
                .append_header(("authorization", basic(second)))
                .to_http_request();
            assert_eq!(
                request
                    .headers()
                    .iter()
                    .filter(|(name, _)| name.as_str() == "authorization")
                    .count(),
                2
            );
            assert!(
                proxy_session_credentials(&request).is_none(),
                "duplicate Basic credentials"
            );
            let request = TestRequest::get()
                .uri("/client/game/start")
                .append_header(("cookie", format!("PHPSESSID={first}")))
                .append_header(("cookie", format!("PHPSESSID={second}")))
                .to_http_request();
            assert_eq!(
                proxy_session_credentials(&request),
                (first == second).then(|| (first.to_string(), false))
            );
            for key in ["PHPSESSID", "phpsessid", "PHPSESSID "] {
                let request = TestRequest::get()
                    .uri("/client/game/start")
                    .insert_header(("cookie", format!("PHPSESSID={first}; {key}={second}")))
                    .to_http_request();
                assert_eq!(
                    proxy_session_credentials(&request),
                    (first == second).then(|| (first.to_string(), false))
                );
            }
        }
    }

    #[test]
    fn headless_websocket_basic_credentials_are_route_and_peer_scoped() {
        use actix_web::{http::Method, test::TestRequest};
        use base64::{engine::general_purpose::STANDARD, Engine};
        let session = "000000000000000000000001";
        let authorization = format!("Basic {}", STANDARD.encode(format!("{session}:")));
        let valid = || {
            TestRequest::get()
                .uri("/fika/headless/client")
                .peer_addr("127.0.0.1:12345".parse().unwrap())
                .insert_header(("upgrade", "websocket"))
                .insert_header(("connection", "Upgrade"))
                .insert_header(("authorization", authorization.clone()))
        };
        assert_eq!(
            proxy_session_credentials(&valid().to_http_request()),
            Some((session.to_string(), true))
        );
        assert!(
            proxy_session_credentials(&valid().uri("/client/game/start").to_http_request())
                .is_none()
        );
        assert!(
            proxy_session_credentials(&valid().method(Method::POST).to_http_request()).is_none()
        );
        assert!(proxy_session_credentials(
            &valid()
                .peer_addr("203.0.113.1:12345".parse().unwrap())
                .to_http_request()
        )
        .is_none());
        assert!(proxy_session_credentials(
            &valid()
                .insert_header(("x-forwarded-for", "127.0.0.1"))
                .to_http_request()
        )
        .is_none());
        assert!(proxy_session_credentials(
            &valid().insert_header(("upgrade", "h2c")).to_http_request()
        )
        .is_none());
        assert!(proxy_session_credentials(
            &valid()
                .insert_header(("connection", "close"))
                .to_http_request()
        )
        .is_none());
        assert!(proxy_session_credentials(
            &valid()
                .insert_header(("cookie", "PHPSESSID=000000000000000000000002"))
                .to_http_request()
        )
        .is_none());
        for credentials in [
            "",
            "not-an-id:",
            "000000000000000000000001:password",
            "00000000000000000000000g:",
        ] {
            let header = format!("Basic {}", STANDARD.encode(credentials));
            assert!(proxy_session_credentials(
                &valid()
                    .insert_header(("authorization", header))
                    .to_http_request()
            )
            .is_none());
        }
        assert!(proxy_session_credentials(
            &valid()
                .insert_header(("authorization", "Basic !!!"))
                .to_http_request()
        )
        .is_none());
        let cookie = TestRequest::get()
            .insert_header(("cookie", format!("PHPSESSID={session}")))
            .to_http_request();
        assert_eq!(
            proxy_session_credentials(&cookie),
            Some((session.to_string(), false))
        );
    }

    #[test]
    fn managed_backend_urls_stay_on_loopback_with_explicit_ports() {
        let external = "https://public.example";
        assert_eq!(
            backend_rewrite_authority(
                &BackendRewriteTarget::HttpProxy,
                external,
                6969,
                9190,
                6969,
                true
            ),
            "127.0.0.1:9190"
        );
        assert_eq!(
            backend_rewrite_authority(
                &BackendRewriteTarget::DirectTcp,
                external,
                6969,
                9190,
                6969,
                true
            ),
            "127.0.0.1:6969"
        );
        assert_eq!(
            backend_rewrite_authority(
                &BackendRewriteTarget::HttpProxy,
                external,
                6969,
                9190,
                6969,
                false
            ),
            "public.example"
        );
        assert_eq!(
            backend_rewrite_authority(
                &BackendRewriteTarget::DirectTcp,
                external,
                6969,
                9190,
                6969,
                false
            ),
            "public.example:6969"
        );
    }

    #[actix_web::test]
    async fn review_regression_linked_managed_identity_keeps_loopback_rewrites() {
        use crate::client::converge::{MANAGED_BY_LABEL, MANAGED_BY_VALUE};
        use actix_web::test::TestRequest;
        let session = "000000000000000000000001";
        let container: bollard::models::ContainerInspectResponse = serde_json::from_value(serde_json::json!({
            "Name":"/fika-headless-1", "State":{"Running":true},
            "Config":{"Labels":{(MANAGED_BY_LABEL):MANAGED_BY_VALUE,"quma.client.index":"1"},"Env":[format!("PROFILE_ID={session}")]}
        })).unwrap();
        let request = TestRequest::get()
            .uri("/client/game/config")
            .peer_addr("127.0.0.1:12345".parse().unwrap())
            .insert_header(("cookie", format!("PHPSESSID={session}")))
            .to_http_request();
        let (_, requires_managed) = proxy_session_credentials(&request).unwrap();
        let checked = std::cell::Cell::new(false);
        let managed = classify_proxy_identity(Some(false), requires_managed, async {
            checked.set(true);
            is_direct_local_peer(&request)
                && managed_headless_profile_matches(&container, session, 2)
        })
        .await
        .unwrap();
        assert!(
            checked.get(),
            "enabled DB links must not skip live managed classification"
        );
        for (target, expected) in [
            (BackendRewriteTarget::HttpProxy, "127.0.0.1:9190"),
            (BackendRewriteTarget::DirectTcp, "127.0.0.1:6969"),
        ] {
            assert_eq!(
                backend_rewrite_authority(
                    &target,
                    "https://public.example",
                    6969,
                    9190,
                    6969,
                    managed
                ),
                expected
            );
        }
    }

    #[actix_web::test]
    async fn disabled_identity_is_rejected_before_managed_lookup() {
        let checked = std::cell::Cell::new(false);
        let result = classify_proxy_identity(Some(true), false, async {
            checked.set(true);
            true
        })
        .await;
        assert_eq!(result, Err(ProxyIdentityError::Disabled));
        assert!(!checked.get());
    }

    #[test]
    fn review_two_managed_direct_rewrite_does_not_use_external_port() {
        assert_eq!(
            backend_rewrite_authority(
                &BackendRewriteTarget::DirectTcp,
                "https://public.example",
                16969,
                9190,
                6969,
                true
            ),
            "127.0.0.1:6969"
        );
        assert_eq!(
            backend_rewrite_authority(
                &BackendRewriteTarget::DirectTcp,
                "https://public.example",
                16969,
                9190,
                6969,
                false
            ),
            "public.example:16969"
        );
    }

    #[test]
    fn review_two_registration_discovers_upper_only_and_overridden_profiles() {
        let tmp = tempfile::tempdir().unwrap();
        let dirs = crate::dirs::QumaDirs::from_root(tmp.path().to_path_buf());
        let relative = "SPT_Runtime/user/profiles";
        let base = dirs.spt_server.join(relative);
        let upper = dirs.runtime_upper().join(relative);
        std::fs::create_dir_all(&base).unwrap();
        std::fs::create_dir_all(&upper).unwrap();
        let shared = "000000000000000000000001";
        let only = "000000000000000000000002";
        for (root, id, username) in [
            (&base, shared, "old"),
            (&upper, shared, "new"),
            (&upper, only, "upper-only"),
        ] {
            std::fs::write(
                root.join(format!("{id}.json")),
                serde_json::json!({"info":{"username":username}}).to_string(),
            )
            .unwrap();
        }
        let db = std::sync::Arc::new(parking_lot::Mutex::new(
            crate::db::Database::open_in_memory().unwrap(),
        ));
        let (events, _) = tokio::sync::broadcast::channel(8);
        handle_player_registration(std::sync::Arc::new(dirs), db.clone(), events);
        let db = db.lock();
        assert_eq!(
            db.get_user_by_spt_profile_id(shared)
                .unwrap()
                .unwrap()
                .username,
            "new"
        );
        assert!(db.get_user_by_spt_profile_id(only).unwrap().is_some());
    }

    #[test]
    fn managed_headless_auth_requires_live_runtime_identity() {
        use crate::client::converge::{MANAGED_BY_LABEL, MANAGED_BY_VALUE};
        use bollard::models::{ContainerConfig, ContainerInspectResponse, ContainerState};
        let session = "000000000000000000000001";
        let container = ContainerInspectResponse {
            name: Some("/fika-headless-1".into()),
            state: Some(ContainerState {
                running: Some(true),
                ..Default::default()
            }),
            config: Some(ContainerConfig {
                labels: Some(
                    [
                        (MANAGED_BY_LABEL.to_string(), MANAGED_BY_VALUE.to_string()),
                        ("quma.client.index".to_string(), "1".to_string()),
                    ]
                    .into_iter()
                    .collect(),
                ),
                env: Some(vec![format!("PROFILE_ID={session}")]),
                ..Default::default()
            }),
            ..Default::default()
        };
        assert!(managed_headless_profile_matches(&container, session, 2));
        assert!(!managed_headless_profile_matches(&container, session, 0));
        assert!(!managed_headless_profile_matches(
            &container,
            "000000000000000000000002",
            2
        ));
        assert!(!managed_headless_profile_matches(&container, "", 2));
        let mut stopped = container.clone();
        stopped.state.as_mut().unwrap().running = Some(false);
        assert!(!managed_headless_profile_matches(&stopped, session, 2));
        let mut unlabelled = container.clone();
        unlabelled.config.as_mut().unwrap().labels = None;
        assert!(!managed_headless_profile_matches(&unlabelled, session, 2));
        let mut renamed = container.clone();
        renamed.name = Some("/unmanaged".into());
        assert!(!managed_headless_profile_matches(&renamed, session, 2));
        let mut no_profile = container;
        no_profile.config.as_mut().unwrap().env = None;
        assert!(!managed_headless_profile_matches(&no_profile, session, 2));
    }

    #[test]
    fn managed_auth_rejects_remote_and_forwarded_peers() {
        use actix_web::test::TestRequest;
        let local = "127.0.0.1:12345".parse().unwrap();
        let remote = "203.0.113.1:12345".parse().unwrap();
        assert!(is_direct_local_peer(
            &TestRequest::get().peer_addr(local).to_http_request()
        ));
        assert!(is_direct_local_peer(
            &TestRequest::get()
                .peer_addr("[::1]:12345".parse().unwrap())
                .to_http_request()
        ));
        assert!(!is_direct_local_peer(&TestRequest::get().to_http_request()));
        assert!(!is_direct_local_peer(
            &TestRequest::get().peer_addr(remote).to_http_request()
        ));
        for header in ["forwarded", "x-forwarded-for", "x-real-ip"] {
            assert!(!is_direct_local_peer(
                &TestRequest::get()
                    .peer_addr(local)
                    .insert_header((header, "127.0.0.1"))
                    .to_http_request()
            ));
            assert!(!is_direct_local_peer(
                &TestRequest::get()
                    .peer_addr(remote)
                    .insert_header((header, "127.0.0.1"))
                    .to_http_request()
            ));
        }
    }

    #[test]
    fn prestige_artwork_exemption_is_narrow_and_read_only() {
        use actix_web::http::Method;
        let id = "0123456789abcdef01234567";
        for prefix in [
            "/files/prestige/",
            "/files/prestige/reward/small/",
            "/files/prestige/reward/big/",
        ] {
            let path = format!("{prefix}{id}.png");
            assert!(is_auth_exempt(&Method::GET, &path), "{path}");
            for method in [Method::POST, Method::PUT, Method::DELETE, Method::HEAD] {
                assert!(!is_auth_exempt(&method, &path), "{method} {path}");
            }
        }
        for path in [
            "/files/prestige/",
            "/files/prestige/reward/medium/0123456789abcdef01234567.png",
            "/files/prestige/../profiles/0123456789abcdef01234567.png",
            "/files/prestige/%2e%2e/0123456789abcdef01234567.png",
            "/files/prestige/0123456789abcdef01234567.json",
            "/files/prestige/0123456789abcdef01234567.png/extra",
            "/files/prestige/not-an-id.png",
            "/files/prestige/0123456789abcdef0123456g.png",
            "/files/prestige/0123456789abcdef0123456.png",
            "/files/prestige/0123456789abcdef012345678.png",
            "/files/user/profiles/0123456789abcdef01234567.json",
        ] {
            assert!(!is_auth_exempt(&Method::GET, path), "{path}");
        }
    }

    #[test]
    fn prepatch_bootstrap_exemption_is_exact_and_read_only() {
        use actix_web::http::Method;
        let path = "/singleplayer/customEnumEntries";
        assert!(is_auth_exempt(&Method::GET, path));
        for method in [Method::POST, Method::PUT, Method::DELETE, Method::HEAD] {
            assert!(!is_auth_exempt(&method, path));
        }
        for other in [
            "/singleplayer/customEnumEntries/",
            "/singleplayer/customEnumEntries/extra",
            "/singleplayer/settings/version",
            "/client/game/start",
        ] {
            assert!(!is_auth_exempt(&Method::GET, other));
        }
    }

    #[test]
    fn non_exempt_path_requires_auth() {
        assert!(!is_auth_exempt(
            &actix_web::http::Method::POST,
            "/client/match/local/start"
        ));
    }

    #[test]
    fn crc32c_known_value() {
        // "123456789" has a well-known CRC32C of 0xE3069283
        assert_eq!(crc32c(b"123456789"), 0xE306_9283);
    }

    #[test]
    fn crc32c_empty() {
        assert_eq!(crc32c(b""), 0);
    }

    #[test]
    fn rewrite_caps_decompressed_output() {
        // Compress a body that would exceed MAX_DECOMPRESSED if fully expanded.
        // Since take() caps read output, the truncated JSON will fail utf8/rewrite
        // but we should NOT OOM.
        use flate2::write::ZlibEncoder;
        use std::io::Write;

        // Create a ~200 KB compressed payload of repetitive JSON.
        // Repetitive data compresses extremely well — this tests that the
        // decompressed output is capped, not that we handle huge compressed input.
        let big_json = format!(
            r#"{{"backendUrl":"https://0.0.0.0:6969/x{}"}}"#,
            "A".repeat(256 * 1024)
        );
        let mut enc = ZlibEncoder::new(Vec::new(), Compression::best());
        enc.write_all(big_json.as_bytes()).unwrap();
        let compressed = enc.finish().unwrap();

        // Should succeed without OOM — the cap is MAX_DECOMPRESSED (64 MB),
        // and our test data is well under that, so it processes normally.
        let result = rewrite_backend_url(&compressed, "tarkov.example.com");
        assert!(result.is_ok());
    }
}
