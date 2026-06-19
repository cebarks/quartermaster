use actix_session::{Session, SessionExt};
use actix_web::body::BoxBody;
use actix_web::dev::{ServiceRequest, ServiceResponse};
use actix_web::middleware::Next;
use actix_web::{web, HttpMessage, HttpRequest, HttpResponse};
use anyhow::Result;
use argon2::password_hash::rand_core::OsRng;
use argon2::password_hash::SaltString;
use argon2::{Argon2, PasswordHash, PasswordHasher, PasswordVerifier};

use crate::db::users::Role;
use crate::web::error::WebError;

pub fn hash_password(password: &str) -> Result<String> {
    let salt = SaltString::generate(&mut OsRng);
    let argon2 = Argon2::default();
    let hash = argon2
        .hash_password(password.as_bytes(), &salt)
        .map_err(|e| anyhow::anyhow!("password hash error: {e}"))?;
    Ok(hash.to_string())
}

pub fn verify_password(password: &str, hash: &str) -> bool {
    let Ok(parsed) = PasswordHash::new(hash) else {
        return false;
    };
    Argon2::default()
        .verify_password(password.as_bytes(), &parsed)
        .is_ok()
}

#[derive(Debug, Clone)]
pub struct SessionUser {
    pub user_id: i64,
    pub username: String,
    pub role: Role,
}

/// Helper to extract SessionUser from session data (mainly for testing)
#[allow(dead_code)]
pub fn get_session_user(session: &Session) -> Option<SessionUser> {
    let user_id = session.get::<i64>("user_id").ok()??;
    let username = session.get::<String>("username").ok()??;
    let role_str = session.get::<String>("role").ok()??;
    let role = Role::try_from(role_str).ok()?;
    Some(SessionUser {
        user_id,
        username,
        role,
    })
}

pub fn require_auth(req: &HttpRequest) -> std::result::Result<SessionUser, WebError> {
    req.extensions()
        .get::<SessionUser>()
        .cloned()
        .ok_or(WebError::Forbidden)
}

pub fn require_capability(
    user: &SessionUser,
    check: fn(&Role) -> bool,
) -> std::result::Result<(), WebError> {
    if !check(&user.role) {
        return Err(WebError::Forbidden);
    }
    Ok(())
}

/// Helper for tests - checks if user has admin capabilities
#[allow(dead_code)]
pub fn require_admin(user: &SessionUser) -> std::result::Result<(), WebError> {
    require_capability(user, Role::can_manage_users)
}

pub fn set_session_user(session: &Session, user: &SessionUser) -> Result<()> {
    session
        .insert("user_id", user.user_id)
        .map_err(|e| anyhow::anyhow!("session error: {e}"))?;
    session
        .insert("username", &user.username)
        .map_err(|e| anyhow::anyhow!("session error: {e}"))?;
    session
        .insert("role", user.role.as_str())
        .map_err(|e| anyhow::anyhow!("session error: {e}"))?;
    Ok(())
}

// -- Auth middleware --

pub async fn auth_middleware(
    req: ServiceRequest,
    next: Next<BoxBody>,
) -> Result<ServiceResponse<BoxBody>, actix_web::Error> {
    let session = req.get_session();
    let user_id: Option<i64> = session.get("user_id").unwrap_or(None);

    let Some(user_id) = user_id else {
        let response = HttpResponse::SeeOther()
            .insert_header(("Location", "/login"))
            .finish();
        return Ok(req.into_response(response).map_into_boxed_body());
    };

    let state = req
        .app_data::<web::Data<crate::web::state::AppState>>()
        .expect("AppState not found")
        .clone();

    let verified_user = web::block(move || {
        let db = state.db.lock();
        db.get_user_by_id(user_id)
    })
    .await;

    let verified_user = match verified_user {
        Ok(Ok(Some(user))) if !user.disabled => user,
        _ => {
            session.purge();
            let response = HttpResponse::SeeOther()
                .insert_header(("Location", "/login"))
                .finish();
            return Ok(req.into_response(response).map_into_boxed_body());
        }
    };

    let session_user = SessionUser {
        user_id: verified_user.id,
        username: verified_user.username.clone(),
        role: verified_user.role,
    };

    let _ = set_session_user(&session, &session_user);
    req.extensions_mut().insert(session_user);
    next.call(req).await
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hash_and_verify() {
        let hash = hash_password("test123").unwrap();
        assert!(verify_password("test123", &hash));
        assert!(!verify_password("wrong", &hash));
    }

    #[test]
    fn verify_invalid_hash() {
        assert!(!verify_password("anything", "not-a-hash"));
    }

    #[test]
    fn session_user_role_capabilities() {
        let admin = SessionUser {
            user_id: 1,
            username: "admin".into(),
            role: Role::Admin,
        };
        let moderator = SessionUser {
            user_id: 2,
            username: "moderator".into(),
            role: Role::Moderator,
        };
        let player = SessionUser {
            user_id: 3,
            username: "player".into(),
            role: Role::Player,
        };

        // Admin can manage users
        assert!(require_admin(&admin).is_ok());
        assert!(require_admin(&moderator).is_err());
        assert!(require_admin(&player).is_err());

        // Admin and moderator can manage mods
        assert!(require_capability(&admin, Role::can_manage_mods).is_ok());
        assert!(require_capability(&moderator, Role::can_manage_mods).is_ok());
        assert!(require_capability(&player, Role::can_manage_mods).is_err());
    }
}
