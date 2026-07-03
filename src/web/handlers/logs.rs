use std::convert::Infallible;
use std::time::Duration;

use actix_session::Session;
use actix_web::web::{Data, Html, Query};
use actix_web::{HttpRequest, HttpResponse};
use actix_web_lab::sse;
use askama::Template;
use bollard::query_parameters::ListContainersOptionsBuilder;
use serde::{Deserialize, Serialize};

use crate::db::logs::{LogQuery as DbLogQuery, StoredLogEntry};
use crate::db::rbac::Permission;
use crate::web::auth::{require_auth, require_permission, SessionUser};
use crate::web::error::WebError;
use crate::web::flash::{take_flash, FlashMessage};
use crate::web::nav::NavContext;
use crate::web::state::AppState;

// Query struct for server (podman) logs — simple limit-only
#[derive(Deserialize)]
pub struct ServerLogQuery {
    limit: Option<usize>,
}

// Query struct for app logs — supports filtering and cursor pagination
#[derive(Deserialize)]
pub struct AppLogQuery {
    level: Option<String>,
    target: Option<String>,
    q: Option<String>,
    before: Option<i64>,
    limit: Option<usize>,
}

// Response for app logs JSON endpoint
#[derive(Serialize)]
pub struct AppLogResponse {
    entries: Vec<StoredLogEntry>,
    has_more: bool,
}

// Query structs for headless client logs
#[derive(Deserialize)]
pub struct HeadlessContainersQuery {
    #[serde(default = "default_true")]
    running_only: bool,
}

fn default_true() -> bool {
    true
}

#[derive(Deserialize)]
pub struct HeadlessLogQuery {
    container: String,
    limit: Option<usize>,
}

#[derive(Deserialize)]
pub struct HeadlessStreamQuery {
    container: String,
}

fn is_valid_headless_name(name: &str) -> bool {
    name.starts_with("fika-headless-") && name["fika-headless-".len()..].parse::<u32>().is_ok()
}

// ---------------------------------------------------------------------------
// App log endpoints — backed by SQLite database with server-side filtering
// ---------------------------------------------------------------------------

pub async fn app_logs_json(
    state: Data<AppState>,
    req: HttpRequest,
    query: Query<AppLogQuery>,
) -> actix_web::Result<HttpResponse> {
    let user = require_auth(&req)?;
    require_permission(&user, Permission::ServerLogs)?;

    let limit = query.limit.unwrap_or(100).min(1000);
    let db_query = DbLogQuery {
        level: query.level.clone(),
        target: query.target.clone(),
        search: query.q.clone(),
        before: query.before,
        limit: limit + 1, // fetch one extra to detect has_more
    };

    let db = state.db.clone();
    let mut entries = actix_web::web::block(move || {
        let db = db.lock();
        db.query_logs(&db_query)
    })
    .await
    .map_err(|e| WebError::Internal(anyhow::anyhow!("{e}")))?
    .map_err(WebError::from)?;

    let has_more = entries.len() > limit;
    entries.truncate(limit);

    Ok(HttpResponse::Ok().json(AppLogResponse { entries, has_more }))
}

pub async fn app_logs_count(
    state: Data<AppState>,
    req: HttpRequest,
) -> actix_web::Result<HttpResponse> {
    let user = require_auth(&req)?;
    require_permission(&user, Permission::ServerLogs)?;

    let counts = state.log_level_counts.read().clone();
    Ok(HttpResponse::Ok().json(counts))
}

pub async fn app_logs_stream(
    state: Data<AppState>,
    req: HttpRequest,
) -> actix_web::Result<sse::Sse<impl futures_util::Stream<Item = Result<sse::Event, Infallible>>>> {
    let user = require_auth(&req)?;
    require_permission(&user, Permission::ServerLogs)?;
    let mut rx = state.log_broadcast.subscribe();

    let (tx, channel_rx) = tokio::sync::mpsc::channel::<sse::Event>(64);

    tokio::spawn(async move {
        use tokio::sync::broadcast::error::RecvError;
        loop {
            let entry = match rx.recv().await {
                Ok(e) => e,
                Err(RecvError::Lagged(_)) => continue,
                Err(RecvError::Closed) => break,
            };

            let mut batch = vec![entry];
            while batch.len() < 50 {
                match rx.try_recv() {
                    Ok(e) => batch.push(e),
                    Err(_) => break,
                }
            }

            for entry in batch {
                if let Ok(json) = serde_json::to_string(&entry) {
                    if tx
                        .send(sse::Event::Data(sse::Data::new(json)))
                        .await
                        .is_err()
                    {
                        return;
                    }
                }
            }

            tokio::time::sleep(Duration::from_millis(250)).await;
        }
    });

    let stream = tokio_stream::wrappers::ReceiverStream::new(channel_rx);
    Ok(sse::Sse::from_infallible_stream(stream).with_keep_alive(Duration::from_secs(15)))
}

// ---------------------------------------------------------------------------
// Server (container) log endpoints — shells out to `podman logs`
// ---------------------------------------------------------------------------

pub async fn server_logs_json(
    state: Data<AppState>,
    req: HttpRequest,
    query: Query<ServerLogQuery>,
) -> actix_web::Result<HttpResponse> {
    let user = require_auth(&req)?;
    require_permission(&user, Permission::ServerLogs)?;
    let container = state
        .config()
        .server_container
        .clone()
        .ok_or(WebError::NotFound)?;
    let tail = query.limit.unwrap_or(100).min(10000);

    let output = tokio::time::timeout(
        Duration::from_secs(10),
        tokio::process::Command::new("podman")
            .args(["logs", "--tail", &tail.to_string(), &container])
            .output(),
    )
    .await
    .map_err(|_| {
        WebError::Internal(anyhow::anyhow!(
            "podman logs timed out (log file may be very large)"
        ))
    })?
    .map_err(|e| WebError::Internal(anyhow::anyhow!("podman logs failed: {e}")))?;

    // Merge stdout and stderr lines
    let mut lines: Vec<String> = String::from_utf8_lossy(&output.stdout)
        .lines()
        .map(String::from)
        .collect();
    let stderr_lines: Vec<String> = String::from_utf8_lossy(&output.stderr)
        .lines()
        .map(String::from)
        .collect();
    lines.extend(stderr_lines);

    Ok(HttpResponse::Ok().json(lines))
}

pub async fn server_logs_stream(
    state: Data<AppState>,
    req: HttpRequest,
) -> actix_web::Result<sse::Sse<impl futures_util::Stream<Item = Result<sse::Event, Infallible>>>> {
    let user = require_auth(&req)?;
    require_permission(&user, Permission::ServerLogs)?;
    let container = state
        .config()
        .server_container
        .clone()
        .ok_or(WebError::NotFound)?;

    let (tx, rx) = tokio::sync::mpsc::channel::<sse::Event>(64);

    tokio::spawn(async move {
        let mut child = match tokio::process::Command::new("podman")
            .args(["logs", "--follow", "--tail", "0", &container])
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped())
            .spawn()
        {
            Ok(c) => c,
            Err(e) => {
                let _ = tx
                    .send(sse::Event::Data(
                        sse::Data::new(format!("error: {e}")).event("error"),
                    ))
                    .await;
                return;
            }
        };

        // Take stdout and stderr before spawning tasks
        let stdout = child.stdout.take();
        let stderr = child.stderr.take();

        // Keep a sender clone to detect when the receiver (SSE client) disconnects.
        // closed() resolves when the rx is dropped, even if readers are blocked on I/O.
        let disconnect = tx.clone();

        let tx_stdout = tx.clone();
        let tx_stderr = tx;

        let stdout_handle = tokio::spawn(async move {
            if let Some(stdout) = stdout {
                use tokio::io::{AsyncBufReadExt, BufReader};
                let mut lines = BufReader::new(stdout).lines();
                while let Ok(Some(line)) = lines.next_line().await {
                    if tx_stdout
                        .send(sse::Event::Data(sse::Data::new(line)))
                        .await
                        .is_err()
                    {
                        break;
                    }
                }
            }
        });

        let stderr_handle = tokio::spawn(async move {
            if let Some(stderr) = stderr {
                use tokio::io::{AsyncBufReadExt, BufReader};
                let mut lines = BufReader::new(stderr).lines();
                while let Ok(Some(line)) = lines.next_line().await {
                    if tx_stderr
                        .send(sse::Event::Data(sse::Data::new(line)))
                        .await
                        .is_err()
                    {
                        break;
                    }
                }
            }
        });

        // Race: either readers finish naturally (process exits) or client disconnects.
        // Without this, readers blocked on next_line() never notice rx was dropped.
        let stdout_abort = stdout_handle.abort_handle();
        let stderr_abort = stderr_handle.abort_handle();
        tokio::select! {
            _ = async { let _ = tokio::join!(stdout_handle, stderr_handle); } => {},
            _ = disconnect.closed() => {
                stdout_abort.abort();
                stderr_abort.abort();
            }
        }

        let _ = child.kill().await;
    });

    let stream = tokio_stream::wrappers::ReceiverStream::new(rx);
    Ok(sse::Sse::from_infallible_stream(stream).with_keep_alive(Duration::from_secs(15)))
}

// ---------------------------------------------------------------------------
// Headless client log endpoints
// ---------------------------------------------------------------------------

pub async fn headless_containers(
    state: Data<AppState>,
    req: HttpRequest,
    query: Query<HeadlessContainersQuery>,
) -> actix_web::Result<HttpResponse> {
    let user = require_auth(&req)?;
    require_permission(&user, Permission::ServerLogs)?;

    let container_mgr = state.container_mgr.as_ref().ok_or(WebError::NotFound)?;

    let label_filter = format!(
        "{}={}",
        crate::client::converge::MANAGED_BY_LABEL,
        crate::client::converge::MANAGED_BY_VALUE,
    );
    let mut filters = std::collections::HashMap::new();
    filters.insert("label", vec![label_filter.as_str()]);
    if query.running_only {
        filters.insert("status", vec!["running"]);
    }

    let containers = container_mgr
        .docker()
        .list_containers(Some(
            ListContainersOptionsBuilder::default()
                .all(!query.running_only)
                .filters(&filters)
                .build(),
        ))
        .await
        .map_err(|e| WebError::Internal(anyhow::anyhow!("{e}")))?;

    let names: Vec<String> = containers
        .into_iter()
        .filter_map(|c| {
            c.names?
                .into_iter()
                .next()
                .map(|n| n.trim_start_matches('/').to_string())
        })
        .collect();

    Ok(HttpResponse::Ok().json(names))
}

pub async fn headless_logs_json(
    state: Data<AppState>,
    req: HttpRequest,
    query: Query<HeadlessLogQuery>,
) -> actix_web::Result<HttpResponse> {
    let user = require_auth(&req)?;
    require_permission(&user, Permission::ServerLogs)?;

    // Ensure podman is available
    let _container_mgr = state.container_mgr.as_ref().ok_or(WebError::NotFound)?;

    if !is_valid_headless_name(&query.container) {
        return Err(WebError::BadRequest("invalid container name".into()).into());
    }

    let container = query.container.clone();
    let tail = query.limit.unwrap_or(100).min(10000);

    let output = tokio::time::timeout(
        Duration::from_secs(10),
        tokio::process::Command::new("podman")
            .args(["logs", "--tail", &tail.to_string(), &container])
            .output(),
    )
    .await
    .map_err(|_| WebError::Internal(anyhow::anyhow!("podman logs timed out")))?
    .map_err(|e| WebError::Internal(anyhow::anyhow!("podman logs failed: {e}")))?;

    let mut lines: Vec<String> = String::from_utf8_lossy(&output.stdout)
        .lines()
        .map(String::from)
        .collect();
    let stderr_lines: Vec<String> = String::from_utf8_lossy(&output.stderr)
        .lines()
        .map(String::from)
        .collect();
    lines.extend(stderr_lines);

    Ok(HttpResponse::Ok().json(lines))
}

pub async fn headless_logs_stream(
    state: Data<AppState>,
    req: HttpRequest,
    query: Query<HeadlessStreamQuery>,
) -> actix_web::Result<sse::Sse<impl futures_util::Stream<Item = Result<sse::Event, Infallible>>>> {
    let user = require_auth(&req)?;
    require_permission(&user, Permission::ServerLogs)?;

    // Ensure podman is available
    let _container_mgr = state.container_mgr.as_ref().ok_or(WebError::NotFound)?;

    if !is_valid_headless_name(&query.container) {
        return Err(WebError::BadRequest("invalid container name".into()).into());
    }

    let container = query.container.clone();
    let (tx, rx) = tokio::sync::mpsc::channel::<sse::Event>(64);

    tokio::spawn(async move {
        let mut child = match tokio::process::Command::new("podman")
            .args(["logs", "--follow", "--tail", "0", &container])
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped())
            .spawn()
        {
            Ok(c) => c,
            Err(e) => {
                let _ = tx
                    .send(sse::Event::Data(
                        sse::Data::new(format!("error: {e}")).event("error"),
                    ))
                    .await;
                return;
            }
        };

        let stdout = child.stdout.take();
        let stderr = child.stderr.take();
        let disconnect = tx.clone();
        let tx_stdout = tx.clone();
        let tx_stderr = tx;

        let stdout_handle = tokio::spawn(async move {
            if let Some(stdout) = stdout {
                use tokio::io::{AsyncBufReadExt, BufReader};
                let mut lines = BufReader::new(stdout).lines();
                while let Ok(Some(line)) = lines.next_line().await {
                    if tx_stdout
                        .send(sse::Event::Data(sse::Data::new(line)))
                        .await
                        .is_err()
                    {
                        break;
                    }
                }
            }
        });

        let stderr_handle = tokio::spawn(async move {
            if let Some(stderr) = stderr {
                use tokio::io::{AsyncBufReadExt, BufReader};
                let mut lines = BufReader::new(stderr).lines();
                while let Ok(Some(line)) = lines.next_line().await {
                    if tx_stderr
                        .send(sse::Event::Data(sse::Data::new(line)))
                        .await
                        .is_err()
                    {
                        break;
                    }
                }
            }
        });

        let stdout_abort = stdout_handle.abort_handle();
        let stderr_abort = stderr_handle.abort_handle();
        tokio::select! {
            _ = async { let _ = tokio::join!(stdout_handle, stderr_handle); } => {},
            _ = disconnect.closed() => {
                stdout_abort.abort();
                stderr_abort.abort();
            }
        }

        let _ = child.kill().await;
    });

    let stream = tokio_stream::wrappers::ReceiverStream::new(rx);
    Ok(sse::Sse::from_infallible_stream(stream).with_keep_alive(Duration::from_secs(15)))
}

// ---------------------------------------------------------------------------
// Log viewer page
// ---------------------------------------------------------------------------

#[derive(Template)]
#[template(path = "logs.html")]
struct LogsTemplate {
    user: SessionUser,
    flash: Option<FlashMessage>,
    csrf_token: String,
    nav: NavContext,
}

pub async fn logs_page(
    state: Data<AppState>,
    req: HttpRequest,
    session: Session,
) -> actix_web::Result<Html> {
    let user = require_auth(&req)?;
    require_permission(&user, Permission::ServerLogs)?;
    let flash = take_flash(&session);
    let csrf_token = crate::web::csrf::get_or_create_token(&session);

    let tmpl = LogsTemplate {
        user,
        flash,
        csrf_token,
        nav: NavContext::from_state(&state),
    };
    Ok(Html::new(tmpl.render().map_err(WebError::from)?))
}
