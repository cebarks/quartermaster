use std::fmt;

use actix_web::HttpResponse;

#[derive(Debug)]
pub enum HeadlessError {
    NotConfigured,
    ClientNotFound(u32),
    ClientInRaid { clients: Vec<u32> },
    MaxClientsReached,
    AlreadyConverging,
    NoFikaClient,
    ContainerError(String),
    ConfigError(String),
    FikaError(String),
    Internal(anyhow::Error),
}

impl fmt::Display for HeadlessError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NotConfigured => write!(f, "Headless clients are not configured"),
            Self::ClientNotFound(i) => write!(f, "Client {i} not found"),
            Self::ClientInRaid { clients } => {
                let ids: Vec<String> = clients.iter().map(|c| c.to_string()).collect();
                write!(f, "Clients {} are in a raid", ids.join(", "))
            }
            Self::MaxClientsReached => write!(f, "Maximum number of headless clients reached"),
            Self::AlreadyConverging => write!(f, "A convergence operation is already in progress"),
            Self::NoFikaClient => write!(f, "Fika client is not available"),
            Self::ContainerError(e) => write!(f, "Container error: {e}"),
            Self::ConfigError(e) => write!(f, "Config error: {e}"),
            Self::FikaError(e) => write!(f, "Fika error: {e}"),
            Self::Internal(e) => write!(f, "Internal error: {e}"),
        }
    }
}

impl std::error::Error for HeadlessError {}

impl actix_web::ResponseError for HeadlessError {
    fn error_response(&self) -> HttpResponse {
        let status = match self {
            Self::NotConfigured | Self::NoFikaClient => {
                actix_web::http::StatusCode::SERVICE_UNAVAILABLE
            }
            Self::ClientNotFound(_) => actix_web::http::StatusCode::NOT_FOUND,
            Self::ClientInRaid { .. } | Self::AlreadyConverging => {
                actix_web::http::StatusCode::CONFLICT
            }
            Self::MaxClientsReached => actix_web::http::StatusCode::BAD_REQUEST,
            Self::ContainerError(_)
            | Self::ConfigError(_)
            | Self::FikaError(_)
            | Self::Internal(_) => actix_web::http::StatusCode::INTERNAL_SERVER_ERROR,
        };
        let body = match self {
            Self::ClientInRaid { clients } => serde_json::json!({
                "error": "client_in_raid",
                "clients": clients,
                "message": self.to_string(),
            }),
            Self::ContainerError(e) | Self::ConfigError(e) | Self::FikaError(e) => {
                tracing::error!(error = %e, error_type = ?self, "Internal headless error");
                serde_json::json!({
                    "error": self.error_code(),
                    "message": "An internal error occurred",
                })
            }
            Self::Internal(e) => {
                tracing::error!(error = %e, "Internal headless error");
                serde_json::json!({
                    "error": self.error_code(),
                    "message": "An internal error occurred",
                })
            }
            _ => serde_json::json!({
                "error": self.error_code(),
                "message": self.to_string(),
            }),
        };
        HttpResponse::build(status).json(body)
    }
}

impl HeadlessError {
    fn error_code(&self) -> &'static str {
        match self {
            Self::NotConfigured => "not_configured",
            Self::ClientNotFound(_) => "client_not_found",
            Self::ClientInRaid { .. } => "client_in_raid",
            Self::MaxClientsReached => "max_clients_reached",
            Self::AlreadyConverging => "already_converging",
            Self::NoFikaClient => "no_fika_client",
            Self::ContainerError(_) => "container_error",
            Self::ConfigError(_) => "config_error",
            Self::FikaError(_) => "fika_error",
            Self::Internal(_) => "internal_error",
        }
    }
}
