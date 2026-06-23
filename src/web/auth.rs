use std::collections::HashSet;

use actix_session::{Session, SessionExt};
use actix_web::body::BoxBody;
use actix_web::dev::{ServiceRequest, ServiceResponse};
use actix_web::middleware::Next;
use actix_web::{web, HttpMessage, HttpRequest, HttpResponse};
use anyhow::Result;
use argon2::password_hash::rand_core::OsRng;
use argon2::password_hash::SaltString;
use argon2::{Argon2, PasswordHash, PasswordHasher, PasswordVerifier};
use chrono;

use crate::db::rbac::Permission;
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
    pub role_name: String,
    pub role_display_name: String,
    pub permissions: HashSet<Permission>,
}

impl SessionUser {
    pub fn has_permission(&self, perm: Permission) -> bool {
        self.permissions.contains(&perm)
    }

    pub fn can(&self, perm_str: &str) -> bool {
        match Permission::from_slug(perm_str) {
            Some(p) => self.permissions.contains(&p),
            None => {
                tracing::warn!(
                    permission = perm_str,
                    "unknown permission slug in can() check"
                );
                false
            }
        }
    }
}

pub fn require_auth(req: &HttpRequest) -> std::result::Result<SessionUser, WebError> {
    req.extensions()
        .get::<SessionUser>()
        .cloned()
        .ok_or(WebError::Forbidden)
}

pub fn require_permission(
    user: &SessionUser,
    perm: Permission,
) -> std::result::Result<(), WebError> {
    if !user.has_permission(perm) {
        return Err(WebError::Forbidden);
    }
    Ok(())
}

pub fn set_session_user(session: &Session, user: &SessionUser) -> Result<()> {
    session
        .insert("user_id", user.user_id)
        .map_err(|e| anyhow::anyhow!("session error: {e}"))?;
    session
        .insert("username", &user.username)
        .map_err(|e| anyhow::anyhow!("session error: {e}"))?;
    session
        .insert("role", &user.role_name)
        .map_err(|e| anyhow::anyhow!("session error: {e}"))?;
    session
        .insert("session_created_at", chrono::Utc::now().to_rfc3339())
        .map_err(|e| anyhow::anyhow!("session error: {e}"))?;
    Ok(())
}

// -- Auth middleware --

pub async fn auth_middleware(
    req: ServiceRequest,
    next: Next<BoxBody>,
) -> Result<ServiceResponse<BoxBody>, actix_web::Error> {
    let session = req.get_session();
    let user_id: Option<i64> = match session.get("user_id") {
        Ok(v) => v,
        Err(e) => {
            tracing::warn!(error = %e, "session deserialization failed");
            None
        }
    };

    let Some(user_id) = user_id else {
        let response = HttpResponse::SeeOther()
            .insert_header(("Location", "/quma/login"))
            .finish();
        return Ok(req.into_response(response).map_into_boxed_body());
    };

    let Some(state) = req
        .app_data::<web::Data<crate::web::state::AppState>>()
        .cloned()
    else {
        tracing::error!("AppState not registered");
        let response = HttpResponse::InternalServerError().finish();
        return Ok(req.into_response(response).map_into_boxed_body());
    };

    let db_result = web::block(move || {
        let db = state.db.lock();
        db.get_user_with_permissions(user_id)
    })
    .await;

    let redirect_login = || {
        HttpResponse::SeeOther()
            .insert_header(("Location", "/quma/login"))
            .finish()
    };

    let (verified_user, role_display, permissions) = match db_result {
        Ok(Ok(Some((user, role_display, permissions)))) if !user.disabled => {
            (user, role_display, permissions)
        }
        Ok(Ok(Some(_))) => {
            tracing::warn!(user_id, "disabled user session purged");
            session.purge();
            return Ok(req.into_response(redirect_login()).map_into_boxed_body());
        }
        Ok(Ok(None)) => {
            tracing::warn!(user_id, "session references nonexistent user");
            session.purge();
            return Ok(req.into_response(redirect_login()).map_into_boxed_body());
        }
        Ok(Err(e)) => {
            tracing::warn!(user_id, error = %e, "auth DB query failed");
            let response = HttpResponse::ServiceUnavailable().finish();
            return Ok(req.into_response(response).map_into_boxed_body());
        }
        Err(e) => {
            tracing::warn!(user_id, error = %e, "auth DB query failed");
            let response = HttpResponse::ServiceUnavailable().finish();
            return Ok(req.into_response(response).map_into_boxed_body());
        }
    };

    let session_user = SessionUser {
        user_id: verified_user.id,
        username: verified_user.username.clone(),
        role_name: verified_user.role.clone(),
        role_display_name: role_display,
        permissions,
    };

    // Check if password was changed after session was created
    if let Some(ref changed_at) = verified_user.password_changed_at {
        let session_created = session.get::<String>("session_created_at").unwrap_or(None);
        let should_invalidate = match session_created {
            None => true, // Old session without timestamp — force re-login
            Some(ref created) => changed_at.as_str() > created.as_str(),
        };
        if should_invalidate {
            tracing::info!(
                user_id = verified_user.id,
                "session invalidated due to password change"
            );
            session.purge();
            return Ok(req.into_response(redirect_login()).map_into_boxed_body());
        }
    }

    let cached_role = session.get::<String>("role").unwrap_or(None);
    if cached_role.as_deref() != Some(&session_user.role_name) {
        if let Err(e) = set_session_user(&session, &session_user) {
            tracing::debug!(user_id, error = %e, "failed to update session cookie");
        }
    }
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
    fn session_user_permission_checks() {
        let mut perms = HashSet::new();
        perms.insert(Permission::UsersManage);
        perms.insert(Permission::ModsInstall);
        perms.insert(Permission::ServerControl);

        let admin = SessionUser {
            user_id: 1,
            username: "admin".into(),
            role_name: "admin".into(),
            role_display_name: "Admin".into(),
            permissions: perms,
        };

        assert!(admin.has_permission(Permission::UsersManage));
        assert!(admin.has_permission(Permission::ModsInstall));
        assert!(!admin.has_permission(Permission::QueueManage));
        assert!(require_permission(&admin, Permission::UsersManage).is_ok());
        assert!(require_permission(&admin, Permission::QueueManage).is_err());

        let player = SessionUser {
            user_id: 2,
            username: "player".into(),
            role_name: "player".into(),
            role_display_name: "Player".into(),
            permissions: HashSet::new(),
        };

        assert!(!player.has_permission(Permission::UsersManage));
        assert!(require_permission(&player, Permission::UsersManage).is_err());
    }

    #[test]
    fn can_method_with_slugs() {
        let mut perms = HashSet::new();
        perms.insert(Permission::ModsInstall);

        let user = SessionUser {
            user_id: 1,
            username: "test".into(),
            role_name: "moderator".into(),
            role_display_name: "Moderator".into(),
            permissions: perms,
        };

        assert!(user.can("mods.install"));
        assert!(!user.can("users.manage"));
        assert!(!user.can("nonexistent.perm"));
    }

    #[test]
    fn session_user_can_checks_permissions() {
        let user = SessionUser {
            user_id: 1,
            username: "test".into(),
            role_name: "custom".into(),
            role_display_name: "Custom".into(),
            permissions: HashSet::from([Permission::ModsInstall, Permission::ServerLogs]),
        };
        assert!(user.can("mods.install"));
        assert!(user.can("server.logs"));
        assert!(!user.can("users.manage"));
        // Unknown permission string returns false (and would log a warning)
        assert!(!user.can("nonexistent.perm"));
    }
}
