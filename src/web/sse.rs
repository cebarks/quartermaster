use actix_session::Session;
use actix_web::web::{self, Data};
use actix_web::HttpResponse;
use futures_util::stream::unfold;
use tokio::sync::broadcast;

use crate::web::auth::require_auth;
use crate::web::state::AppState;

#[derive(Clone, Debug)]
pub enum ServerEvent {
    TaskChanged,
    ModsChanged,
}

pub async fn events_stream(
    state: Data<AppState>,
    session: Session,
) -> actix_web::Result<HttpResponse> {
    require_auth(&session)?;

    let rx = state.events.subscribe();

    let stream = unfold(rx, |mut rx| async move {
        loop {
            match rx.recv().await {
                Ok(event) => {
                    let msg = match event {
                        ServerEvent::TaskChanged => "event: taskChanged\ndata: \n\n",
                        ServerEvent::ModsChanged => "event: modsChanged\ndata: \n\n",
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
