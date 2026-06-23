use std::convert::Infallible;
use std::time::Duration;

use actix_session::Session;
use actix_web::web::{Data, Html, Query};
use actix_web::{HttpRequest, HttpResponse};
use actix_web_lab::sse;
use askama::Template;
use serde::Deserialize;
use tokio::sync::broadcast::error::RecvError;

use crate::db::rbac::Permission;
use crate::web::auth::{require_auth, require_permission, SessionUser};
use crate::web::error::WebError;
use crate::web::flash::{take_flash, FlashMessage};
use crate::web::nav::NavContext;
use crate::web::state::AppState;

#[derive(Deserialize)]
pub struct LogQuery {
    limit: Option<usize>,
}

// ---------------------------------------------------------------------------
// App log endpoints — backed by the in-process LogBroadcast ring buffer
// ---------------------------------------------------------------------------

pub async fn app_logs_json(
    state: Data<AppState>,
    req: HttpRequest,
    query: Query<LogQuery>,
) -> actix_web::Result<HttpResponse> {
    let user = require_auth(&req)?;
    require_permission(&user, Permission::ServerLogs)?;
    let limit = query.limit.unwrap_or(100).min(10000);
    let entries = state.log_broadcast.recent(limit);
    Ok(HttpResponse::Ok().json(entries))
}

pub async fn app_logs_stream(
    state: Data<AppState>,
    req: HttpRequest,
) -> actix_web::Result<sse::Sse<impl futures_util::Stream<Item = Result<sse::Event, Infallible>>>> {
    let user = require_auth(&req)?;
    require_permission(&user, Permission::ServerLogs)?;
    let mut rx = state.log_broadcast.subscribe();

    let stream = async_stream::stream! {
        loop {
            match rx.recv().await {
                Ok(entry) => {
                    if let Ok(json) = serde_json::to_string(&entry) {
                        yield Ok(sse::Event::Data(sse::Data::new(json)));
                    }
                }
                Err(RecvError::Lagged(n)) => {
                    let comment: bytestring::ByteString = format!("lagged:{n}").into();
                    yield Ok(sse::Event::Comment(comment));
                }
                Err(RecvError::Closed) => break,
            }
        }
    };

    Ok(sse::Sse::from_stream(stream).with_keep_alive(Duration::from_secs(15)))
}

// ---------------------------------------------------------------------------
// Server (container) log endpoints — shells out to `podman logs`
// ---------------------------------------------------------------------------

pub async fn server_logs_json(
    state: Data<AppState>,
    req: HttpRequest,
    query: Query<LogQuery>,
) -> actix_web::Result<HttpResponse> {
    let user = require_auth(&req)?;
    require_permission(&user, Permission::ServerLogs)?;
    let container = state
        .config()
        .server_container
        .clone()
        .ok_or(WebError::NotFound)?;
    let tail = query.limit.unwrap_or(100).min(10000);

    let output = tokio::process::Command::new("podman")
        .args(["logs", "--tail", &tail.to_string(), &container])
        .output()
        .await
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
