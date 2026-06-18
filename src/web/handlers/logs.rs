use std::convert::Infallible;
use std::time::Duration;

use actix_session::Session;
use actix_web::web::{Data, Query};
use actix_web::HttpResponse;
use actix_web_lab::sse;
use serde::Deserialize;
use tokio::sync::broadcast::error::RecvError;

use crate::web::auth::require_auth;
use crate::web::error::WebError;
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
    session: Session,
    query: Query<LogQuery>,
) -> actix_web::Result<HttpResponse> {
    require_auth(&session)?;
    let limit = query.limit.unwrap_or(100);
    let entries = state.log_broadcast.recent(limit);
    Ok(HttpResponse::Ok().json(entries))
}

pub async fn app_logs_stream(
    state: Data<AppState>,
    session: Session,
) -> actix_web::Result<sse::Sse<impl futures_util::Stream<Item = Result<sse::Event, Infallible>>>> {
    require_auth(&session)?;
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
    session: Session,
    query: Query<LogQuery>,
) -> actix_web::Result<HttpResponse> {
    require_auth(&session)?;
    let container = state
        .config
        .server_container
        .as_deref()
        .ok_or(WebError::NotFound)?;
    let tail = query.limit.unwrap_or(100);

    let output = tokio::process::Command::new("podman")
        .args(["logs", "--tail", &tail.to_string(), container])
        .output()
        .await
        .map_err(|e| WebError::Internal(anyhow::anyhow!("podman logs failed: {e}")))?;

    let lines: Vec<String> = String::from_utf8_lossy(&output.stdout)
        .lines()
        .map(String::from)
        .collect();
    Ok(HttpResponse::Ok().json(lines))
}

pub async fn server_logs_stream(
    state: Data<AppState>,
    session: Session,
) -> actix_web::Result<sse::Sse<impl futures_util::Stream<Item = Result<sse::Event, Infallible>>>> {
    require_auth(&session)?;
    let container = state
        .config
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

        if let Some(stdout) = child.stdout.take() {
            use tokio::io::{AsyncBufReadExt, BufReader};
            let mut lines = BufReader::new(stdout).lines();
            while let Ok(Some(line)) = lines.next_line().await {
                if tx
                    .send(sse::Event::Data(sse::Data::new(line)))
                    .await
                    .is_err()
                {
                    break;
                }
            }
        }

        let _ = child.kill().await;
    });

    let stream = tokio_stream::wrappers::ReceiverStream::new(rx);
    Ok(sse::Sse::from_infallible_stream(stream).with_keep_alive(Duration::from_secs(15)))
}
