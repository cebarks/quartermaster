use actix_session::{Session, SessionExt};
use actix_web::body::{BoxBody, MessageBody};
use actix_web::dev::{Service, ServiceRequest, ServiceResponse, Transform};
use actix_web::{Error, HttpResponse};
use anyhow::Result;
use argon2::password_hash::rand_core::OsRng;
use argon2::password_hash::SaltString;
use argon2::{Argon2, PasswordHash, PasswordHasher, PasswordVerifier};

use std::future::{ready, Future, Ready};
use std::pin::Pin;
use std::task::{Context, Poll};

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
    pub role: String,
}

impl SessionUser {
    pub fn is_admin(&self) -> bool {
        self.role == "admin"
    }
}

pub fn get_session_user(session: &Session) -> Option<SessionUser> {
    let user_id = session.get::<i64>("user_id").ok()??;
    let username = session.get::<String>("username").ok()??;
    let role = session.get::<String>("role").ok()??;
    Some(SessionUser {
        user_id,
        username,
        role,
    })
}

pub fn set_session_user(session: &Session, user: &SessionUser) -> Result<()> {
    session
        .insert("user_id", user.user_id)
        .map_err(|e| anyhow::anyhow!("session error: {e}"))?;
    session
        .insert("username", &user.username)
        .map_err(|e| anyhow::anyhow!("session error: {e}"))?;
    session
        .insert("role", &user.role)
        .map_err(|e| anyhow::anyhow!("session error: {e}"))?;
    Ok(())
}

// -- RequireAuth middleware --

pub struct RequireAuth;

impl<S, B> Transform<S, ServiceRequest> for RequireAuth
where
    S: Service<ServiceRequest, Response = ServiceResponse<B>, Error = Error> + 'static,
    B: MessageBody + 'static,
{
    type Response = ServiceResponse<BoxBody>;
    type Error = Error;
    type Transform = RequireAuthMiddleware<S>;
    type InitError = ();
    type Future = Ready<Result<Self::Transform, Self::InitError>>;

    fn new_transform(&self, service: S) -> Self::Future {
        ready(Ok(RequireAuthMiddleware { service }))
    }
}

pub struct RequireAuthMiddleware<S> {
    service: S,
}

impl<S, B> Service<ServiceRequest> for RequireAuthMiddleware<S>
where
    S: Service<ServiceRequest, Response = ServiceResponse<B>, Error = Error> + 'static,
    B: MessageBody + 'static,
{
    type Response = ServiceResponse<BoxBody>;
    type Error = Error;
    type Future = Pin<Box<dyn Future<Output = Result<Self::Response, Self::Error>>>>;

    fn poll_ready(&self, cx: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
        self.service.poll_ready(cx)
    }

    fn call(&self, req: ServiceRequest) -> Self::Future {
        let session = req.get_session();
        let user = get_session_user(&session);

        if user.is_none() {
            return Box::pin(async move {
                let resp = HttpResponse::SeeOther()
                    .insert_header(("Location", "/login"))
                    .finish();
                Ok(req.into_response(resp).map_into_boxed_body())
            });
        }

        let fut = self.service.call(req);
        Box::pin(async move { fut.await.map(|res| res.map_into_boxed_body()) })
    }
}

// -- RequireAdmin middleware --

pub struct RequireAdmin;

impl<S, B> Transform<S, ServiceRequest> for RequireAdmin
where
    S: Service<ServiceRequest, Response = ServiceResponse<B>, Error = Error> + 'static,
    B: MessageBody + 'static,
{
    type Response = ServiceResponse<BoxBody>;
    type Error = Error;
    type Transform = RequireAdminMiddleware<S>;
    type InitError = ();
    type Future = Ready<Result<Self::Transform, Self::InitError>>;

    fn new_transform(&self, service: S) -> Self::Future {
        ready(Ok(RequireAdminMiddleware { service }))
    }
}

pub struct RequireAdminMiddleware<S> {
    service: S,
}

impl<S, B> Service<ServiceRequest> for RequireAdminMiddleware<S>
where
    S: Service<ServiceRequest, Response = ServiceResponse<B>, Error = Error> + 'static,
    B: MessageBody + 'static,
{
    type Response = ServiceResponse<BoxBody>;
    type Error = Error;
    type Future = Pin<Box<dyn Future<Output = Result<Self::Response, Self::Error>>>>;

    fn poll_ready(&self, cx: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
        self.service.poll_ready(cx)
    }

    fn call(&self, req: ServiceRequest) -> Self::Future {
        let session = req.get_session();
        let user = get_session_user(&session);

        match user {
            None => Box::pin(async move {
                let resp = HttpResponse::SeeOther()
                    .insert_header(("Location", "/login"))
                    .finish();
                Ok(req.into_response(resp).map_into_boxed_body())
            }),
            Some(u) if !u.is_admin() => Box::pin(async move {
                let resp = HttpResponse::Forbidden().body("admin access required");
                Ok(req.into_response(resp).map_into_boxed_body())
            }),
            Some(_) => {
                let fut = self.service.call(req);
                Box::pin(async move { fut.await.map(|res| res.map_into_boxed_body()) })
            }
        }
    }
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
    fn session_user_admin_check() {
        let admin = SessionUser {
            user_id: 1,
            username: "admin".into(),
            role: "admin".into(),
        };
        let player = SessionUser {
            user_id: 2,
            username: "player".into(),
            role: "player".into(),
        };
        assert!(admin.is_admin());
        assert!(!player.is_admin());
    }
}
