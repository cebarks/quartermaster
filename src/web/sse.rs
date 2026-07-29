use std::convert::Infallible;
use std::sync::atomic::Ordering;
use std::time::Duration;

use actix_web::web::Data;
use actix_web::HttpRequest;
use actix_web_lab::sse;
use tokio::sync::broadcast;

use crate::web::auth::require_auth;
use crate::web::state::AppState;

/// ponytail: 128 concurrent SSE connections; single-server tool won't need more.
/// Raise if needed — this just prevents runaway tab accumulation.
const MAX_SSE_CONNECTIONS: usize = 128;

/// Drop guard that decrements the SSE connection counter when the stream task ends.
/// Holds a `Data<AppState>` (which is `Arc<AppState>`) so the counter reference stays valid.
struct SseConnectionGuard(Data<AppState>);

impl Drop for SseConnectionGuard {
    fn drop(&mut self) {
        self.0.sse_connections.fetch_sub(1, Ordering::Relaxed);
    }
}

#[derive(Clone, Debug)]
pub enum ServerEvent {
    TaskChanged,
    ModsChanged,
    ServerTransition,
    PlayerRegistered,
    RaidStarted,
    RaidEnded,
    IntegrityChanged,
    HeadlessChanged,
}

impl ServerEvent {
    fn event_name(&self) -> &'static str {
        match self {
            Self::TaskChanged => "taskChanged",
            Self::ModsChanged => "modsChanged",
            Self::ServerTransition => "serverStateChanged",
            Self::PlayerRegistered => "playerRegistered",
            Self::RaidStarted => "raidStarted",
            Self::RaidEnded => "raidEnded",
            Self::IntegrityChanged => "integrityChanged",
            Self::HeadlessChanged => "headlessChanged",
        }
    }
}

pub async fn events_stream(
    state: Data<AppState>,
    req: HttpRequest,
) -> actix_web::Result<sse::Sse<impl futures_util::Stream<Item = Result<sse::Event, Infallible>>>> {
    require_auth(&req)?;

    // Relaxed: soft cap, brief overshoot is harmless
    let prev = state.sse_connections.fetch_add(1, Ordering::Relaxed);
    if prev >= MAX_SSE_CONNECTIONS {
        state.sse_connections.fetch_sub(1, Ordering::Relaxed);
        return Err(actix_web::error::ErrorTooManyRequests(
            "too many SSE connections",
        ));
    }
    let guard = SseConnectionGuard(state.clone());

    let mut rx = state.events.subscribe();
    let (tx, channel_rx) = tokio::sync::mpsc::channel::<sse::Event>(64);

    tokio::spawn(async move {
        let _guard = guard;
        loop {
            match rx.recv().await {
                Ok(event) => {
                    let sse_event = sse::Event::Data(sse::Data::new("").event(event.event_name()));
                    if tx.send(sse_event).await.is_err() {
                        break;
                    }
                }
                Err(broadcast::error::RecvError::Lagged(_)) => continue,
                Err(broadcast::error::RecvError::Closed) => break,
            }
        }
    });

    let stream = tokio_stream::wrappers::ReceiverStream::new(channel_rx);
    // ponytail: 30s keepalive prevents proxies from closing idle connections
    Ok(sse::Sse::from_infallible_stream(stream).with_keep_alive(Duration::from_secs(30)))
}
