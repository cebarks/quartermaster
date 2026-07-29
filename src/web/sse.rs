use std::convert::Infallible;
use std::time::Duration;

use actix_web::web::Data;
use actix_web::HttpRequest;
use actix_web_lab::sse;
use tokio::sync::broadcast;

use crate::web::auth::require_auth;
use crate::web::state::AppState;

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

    let mut rx = state.events.subscribe();
    let (tx, channel_rx) = tokio::sync::mpsc::channel::<sse::Event>(64);

    tokio::spawn(async move {
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
