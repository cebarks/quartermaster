use actix_web::http::StatusCode;
use actix_web::{HttpResponse, ResponseError};

#[derive(Debug)]
pub enum WebError {
    Internal(anyhow::Error),
    NotFound,
    Forbidden,
    BadRequest(String),
}

impl std::fmt::Display for WebError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            WebError::Internal(e) => write!(f, "internal error: {e}"),
            WebError::NotFound => write!(f, "not found"),
            WebError::Forbidden => write!(f, "forbidden"),
            WebError::BadRequest(msg) => write!(f, "bad request: {msg}"),
        }
    }
}

impl ResponseError for WebError {
    fn status_code(&self) -> StatusCode {
        match self {
            WebError::Internal(_) => StatusCode::INTERNAL_SERVER_ERROR,
            WebError::NotFound => StatusCode::NOT_FOUND,
            WebError::Forbidden => StatusCode::FORBIDDEN,
            WebError::BadRequest(_) => StatusCode::BAD_REQUEST,
        }
    }

    fn error_response(&self) -> HttpResponse {
        if let WebError::Internal(e) = self {
            eprintln!("internal error: {e:#}");
        }
        let body = match self {
            WebError::Internal(_) => "an internal server error occurred".to_string(),
            WebError::NotFound => "not found".to_string(),
            WebError::Forbidden => "forbidden".to_string(),
            WebError::BadRequest(msg) => format!("bad request: {msg}"),
        };
        HttpResponse::build(self.status_code()).body(body)
    }
}

impl From<anyhow::Error> for WebError {
    fn from(e: anyhow::Error) -> Self {
        WebError::Internal(e)
    }
}

impl From<rusqlite::Error> for WebError {
    fn from(e: rusqlite::Error) -> Self {
        WebError::Internal(e.into())
    }
}

impl From<askama::Error> for WebError {
    fn from(e: askama::Error) -> Self {
        WebError::Internal(e.into())
    }
}

impl From<actix_web::error::BlockingError> for WebError {
    fn from(e: actix_web::error::BlockingError) -> Self {
        WebError::Internal(anyhow::anyhow!("blocking error: {e}"))
    }
}
