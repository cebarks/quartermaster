use std::sync::Arc;

use actix_web::web::{self, Data};
use actix_web::{HttpRequest, HttpResponse};
use futures_util::{SinkExt, StreamExt};
use tokio_tungstenite::tungstenite;

use crate::web::state::AppState;

/// WebSocket proxy handler. Accepts a client WebSocket upgrade, opens a WSS
/// connection to the upstream SPT server, and bridges frames bidirectionally.
///
/// Uses `actix_web::rt::spawn` (spawn_local) because `actix_ws::MessageStream`
/// is `!Send`. Both directions are driven in a single `tokio::select!` loop
/// since spawn_local runs on the actix-web runtime's local set.
pub async fn ws_proxy_handler(
    req: HttpRequest,
    body: web::Payload,
    state: Data<AppState>,
) -> actix_web::Result<HttpResponse> {
    let path = req
        .uri()
        .path_and_query()
        .map(|pq| pq.as_str())
        .unwrap_or(req.path());
    let (host, port) = crate::server_detect::resolve_server_addr(&state.config(), &state.spt_dir);
    let upstream_url = format!("wss://{}:{}{}", host, port, path);

    let (response, mut client_session, mut client_stream) =
        actix_ws::handle(&req, body).map_err(|e| {
            tracing::error!(error = %e, "failed to accept WebSocket");
            actix_web::error::ErrorBadRequest("WebSocket handshake failed")
        })?;

    let path_owned = path.to_string();
    let state_clone = state.clone();

    actix_web::rt::spawn(async move {
        state_clone.proxy_metrics.increment_ws_connections();

        let connector = tokio_tungstenite::Connector::Rustls(Arc::new(insecure_tls_config()));

        let ws_result = tokio_tungstenite::connect_async_tls_with_config(
            &upstream_url,
            None,
            false,
            Some(connector),
        )
        .await;

        let (upstream_ws, _) = match ws_result {
            Ok(conn) => conn,
            Err(e) => {
                tracing::error!(error = %e, url = %upstream_url, "failed to connect WebSocket upstream");
                let _ = client_session.close(None).await;
                state_clone.proxy_metrics.decrement_ws_connections();
                return;
            }
        };

        tracing::info!(path = %path_owned, "WebSocket proxy connected");

        let (mut upstream_sink, mut upstream_stream) = upstream_ws.split();

        // Drive both directions in a single select loop.
        // When either side closes or errors, we tear down the connection.
        // `client_closed` tracks half-close from the client side — after the
        // client disconnects we keep draining upstream until it closes too.
        // Upstream close always causes a full exit (we close the client session).
        let mut client_closed = false;

        loop {
            tokio::select! {
                // Client → upstream
                client_msg = client_stream.next(), if !client_closed => {
                    match client_msg {
                        Some(Ok(actix_ws::Message::Binary(data))) => {
                            if upstream_sink.send(tungstenite::Message::Binary(data.to_vec().into())).await.is_err() {
                                break;
                            }
                        }
                        Some(Ok(actix_ws::Message::Text(text))) => {
                            if upstream_sink.send(tungstenite::Message::text(text.to_string())).await.is_err() {
                                break;
                            }
                        }
                        Some(Ok(actix_ws::Message::Ping(data))) => {
                            if upstream_sink.send(tungstenite::Message::Ping(data.to_vec().into())).await.is_err() {
                                break;
                            }
                        }
                        Some(Ok(actix_ws::Message::Pong(data))) => {
                            if upstream_sink.send(tungstenite::Message::Pong(data.to_vec().into())).await.is_err() {
                                break;
                            }
                        }
                        Some(Ok(actix_ws::Message::Close(_))) | Some(Err(_)) | None => {
                            let _ = upstream_sink.send(tungstenite::Message::Close(None)).await;
                            client_closed = true;
                        }
                        // Continuation and Nop — skip
                        _ => {}
                    }
                }
                // Upstream → client
                upstream_msg = upstream_stream.next() => {
                    match upstream_msg {
                        Some(Ok(tungstenite::Message::Binary(data))) => {
                            if client_session.binary(data).await.is_err() {
                                break;
                            }
                        }
                        Some(Ok(tungstenite::Message::Text(text))) => {
                            if client_session.text(text.to_string()).await.is_err() {
                                break;
                            }
                        }
                        Some(Ok(tungstenite::Message::Ping(data))) => {
                            if client_session.ping(&data).await.is_err() {
                                break;
                            }
                        }
                        Some(Ok(tungstenite::Message::Pong(data))) => {
                            if client_session.pong(&data).await.is_err() {
                                break;
                            }
                        }
                        Some(Ok(tungstenite::Message::Close(_))) | Some(Err(_)) | None => {
                            let _ = client_session.close(None).await;
                            break;
                        }
                        // Frame variant — skip
                        _ => {}
                    }
                }
            }
        }

        state_clone.proxy_metrics.decrement_ws_connections();
        tracing::info!(path = %path_owned, "WebSocket proxy disconnected");
    });

    Ok(response)
}

/// Build a rustls `ClientConfig` that accepts any server certificate.
/// Required because the SPT server uses a self-signed certificate.
fn insecure_tls_config() -> rustls::ClientConfig {
    rustls::ClientConfig::builder()
        .dangerous()
        .with_custom_certificate_verifier(Arc::new(InsecureCertVerifier))
        .with_no_client_auth()
}

#[derive(Debug)]
struct InsecureCertVerifier;

impl rustls::client::danger::ServerCertVerifier for InsecureCertVerifier {
    fn verify_server_cert(
        &self,
        _end_entity: &rustls::pki_types::CertificateDer<'_>,
        _intermediates: &[rustls::pki_types::CertificateDer<'_>],
        _server_name: &rustls::pki_types::ServerName<'_>,
        _ocsp_response: &[u8],
        _now: rustls::pki_types::UnixTime,
    ) -> Result<rustls::client::danger::ServerCertVerified, rustls::Error> {
        Ok(rustls::client::danger::ServerCertVerified::assertion())
    }

    fn verify_tls12_signature(
        &self,
        _message: &[u8],
        _cert: &rustls::pki_types::CertificateDer<'_>,
        _dss: &rustls::DigitallySignedStruct,
    ) -> Result<rustls::client::danger::HandshakeSignatureValid, rustls::Error> {
        Ok(rustls::client::danger::HandshakeSignatureValid::assertion())
    }

    fn verify_tls13_signature(
        &self,
        _message: &[u8],
        _cert: &rustls::pki_types::CertificateDer<'_>,
        _dss: &rustls::DigitallySignedStruct,
    ) -> Result<rustls::client::danger::HandshakeSignatureValid, rustls::Error> {
        Ok(rustls::client::danger::HandshakeSignatureValid::assertion())
    }

    fn supported_verify_schemes(&self) -> Vec<rustls::SignatureScheme> {
        vec![
            rustls::SignatureScheme::RSA_PKCS1_SHA256,
            rustls::SignatureScheme::RSA_PKCS1_SHA384,
            rustls::SignatureScheme::RSA_PKCS1_SHA512,
            rustls::SignatureScheme::ECDSA_NISTP256_SHA256,
            rustls::SignatureScheme::ECDSA_NISTP384_SHA384,
            rustls::SignatureScheme::ECDSA_NISTP521_SHA512,
            rustls::SignatureScheme::RSA_PSS_SHA256,
            rustls::SignatureScheme::RSA_PSS_SHA384,
            rustls::SignatureScheme::RSA_PSS_SHA512,
            rustls::SignatureScheme::ED25519,
            rustls::SignatureScheme::ED448,
        ]
    }
}
