use actix_web::http::StatusCode;
use actix_web::{HttpResponse, ResponseError};
use askama::Template;

use crate::web::flash::FlashMessage;

#[derive(Template)]
#[template(path = "error.html")]
struct ErrorTemplate {
    title: String,
    message: String,
    flash: Option<FlashMessage>,
}

#[derive(Debug)]
pub enum WebError {
    Internal(anyhow::Error),
    NotFound,
    Forbidden,
    BadRequest(String),
    UnprocessableEntity(String),
}

impl std::fmt::Display for WebError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            WebError::Internal(e) => write!(f, "internal error: {e}"),
            WebError::NotFound => write!(f, "not found"),
            WebError::Forbidden => write!(f, "forbidden"),
            WebError::BadRequest(msg) => write!(f, "bad request: {msg}"),
            WebError::UnprocessableEntity(msg) => write!(f, "unprocessable: {msg}"),
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
            WebError::UnprocessableEntity(_) => StatusCode::UNPROCESSABLE_ENTITY,
        }
    }

    fn error_response(&self) -> HttpResponse {
        if let WebError::Internal(e) = self {
            tracing::error!(err = %e, "internal server error");
        }

        let (title, message) = match self {
            WebError::Internal(_) => (
                "Internal Server Error".to_string(),
                "An unexpected error occurred. Please try again.".to_string(),
            ),
            WebError::NotFound => (
                "Not Found".to_string(),
                "The page you're looking for doesn't exist.".to_string(),
            ),
            WebError::Forbidden => (
                "Forbidden".to_string(),
                "You don't have permission to access this page.".to_string(),
            ),
            WebError::BadRequest(msg) => ("Bad Request".to_string(), msg.clone()),
            WebError::UnprocessableEntity(msg) => ("Unprocessable Entity".to_string(), msg.clone()),
        };

        let tmpl = ErrorTemplate {
            title: title.clone(),
            message,
            flash: None,
        };
        match tmpl.render() {
            Ok(body) => HttpResponse::build(self.status_code())
                .content_type("text/html")
                .body(body),
            Err(_) => HttpResponse::build(self.status_code()).body(title),
        }
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

impl From<std::io::Error> for WebError {
    fn from(e: std::io::Error) -> Self {
        WebError::Internal(e.into())
    }
}

impl From<serde_json::Error> for WebError {
    fn from(e: serde_json::Error) -> Self {
        WebError::BadRequest(format!("Invalid JSON: {e}"))
    }
}
