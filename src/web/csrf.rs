use actix_session::Session;
use rand::RngExt;
use subtle::ConstantTimeEq;

const CSRF_SESSION_KEY: &str = "csrf_token";
const CSRF_PREV_KEY: &str = "csrf_token_prev";
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
    let current_valid = match session.get::<String>(CSRF_SESSION_KEY) {
        Ok(Some(ref session_token)) => session_token.as_bytes().ct_eq(form_token.as_bytes()).into(),
        _ => false,
    };
    let prev_valid = match session.get::<String>(CSRF_PREV_KEY) {
        Ok(Some(ref prev_token)) => prev_token.as_bytes().ct_eq(form_token.as_bytes()).into(),
        _ => false,
    };

    if current_valid || prev_valid {
        if let Ok(Some(current)) = session.get::<String>(CSRF_SESSION_KEY) {
            let _ = session.insert(CSRF_PREV_KEY, &current);
        }
        session.remove(CSRF_SESSION_KEY);
        true
    } else {
        false
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
    use actix_session::SessionExt;

    fn test_session() -> Session {
        let req = actix_web::test::TestRequest::default().to_http_request();
        req.get_session()
    }

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

    #[test]
    fn token_rotated_after_validation() {
        let session = test_session();
        let token = get_or_create_token(&session);
        assert!(validate_token(&session, &token));
        // Second use — hits the prev window
        assert!(validate_token(&session, &token));
        // Get the new current token and validate it to push the original out
        let new_token = get_or_create_token(&session);
        assert!(validate_token(&session, &new_token));
        // Original is now evicted
        assert!(!validate_token(&session, &token));
    }

    #[test]
    fn invalid_token_rejected() {
        let session = test_session();
        let _token = get_or_create_token(&session);
        assert!(!validate_token(&session, "wrong-token"));
    }

    #[test]
    fn empty_session_rejects() {
        let session = test_session();
        assert!(!validate_token(&session, "anything"));
    }
}
