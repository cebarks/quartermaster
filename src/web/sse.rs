use actix_web::web::{self, Data};
use actix_web::{HttpRequest, HttpResponse};
use futures_util::stream::unfold;
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
}

pub async fn events_stream(
    state: Data<AppState>,
    req: HttpRequest,
) -> actix_web::Result<HttpResponse> {
    require_auth(&req)?;

    let rx = state.events.subscribe();

    let stream = unfold(rx, |mut rx| async move {
        loop {
            match rx.recv().await {
                Ok(event) => {
                    let msg = match event {
                        ServerEvent::TaskChanged => "event: taskChanged\ndata: \n\n",
                        ServerEvent::ModsChanged => "event: modsChanged\ndata: \n\n",
                        ServerEvent::ServerTransition => "event: serverStateChanged\ndata: \n\n",
                        ServerEvent::PlayerRegistered => "event: playerRegistered\ndata: \n\n",
                        ServerEvent::RaidStarted => "event: raidStarted\ndata: \n\n",
                        ServerEvent::RaidEnded => "event: raidEnded\ndata: \n\n",
                        ServerEvent::IntegrityChanged => "event: integrityChanged\ndata: \n\n",
                    };
                    return Some((Ok::<_, actix_web::Error>(web::Bytes::from(msg)), rx));
                }
                Err(broadcast::error::RecvError::Lagged(_)) => continue,
                Err(broadcast::error::RecvError::Closed) => return None,
            }
        }
    });

    Ok(HttpResponse::Ok()
        .content_type("text/event-stream")
        .insert_header(("Cache-Control", "no-cache"))
        .streaming(stream))
}
