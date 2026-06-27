use actix_session::Session;
use rand::RngExt;

const CSRF_SESSION_KEY: &str = "csrf_token";
const TOKEN_LEN: usize = 32;

pub fn get_or_create_token(session: &Session) -> String {
    if let Ok(Some(token)) = session.get::<String>(CSRF_SESSION_KEY) {
        return token;
    }
    let token: String = rand::rng()
        .sample_iter(&rand::distr::Alphanumeric)
        .take(TOKEN_LEN)
        .map(char::from)
        .collect();
    if let Err(e) = session.insert(CSRF_SESSION_KEY, &token) {
        tracing::warn!(err = %e, "failed to insert CSRF token into session");
    }
    token
}

pub fn validate_token(session: &Session, form_token: &str) -> bool {
    match session.get::<String>(CSRF_SESSION_KEY) {
        Ok(Some(session_token)) => session_token == form_token,
        _ => false,
    }
}

#[derive(serde::Deserialize)]
pub struct CsrfForm {
    pub csrf_token: String,
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;

    #[test]
    fn token_length_and_charset() {
        let token: String = rand::rng()
            .sample_iter(&rand::distr::Alphanumeric)
            .take(TOKEN_LEN)
            .map(char::from)
            .collect();
        assert_eq!(token.len(), TOKEN_LEN);
        assert!(token.chars().all(|c| c.is_ascii_alphanumeric()));
    }
}
