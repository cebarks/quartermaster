use actix_session::Session;
use actix_web::web::{self, Data, Form, Query};
use actix_web::HttpResponse;
use askama::Template;
use subtle::ConstantTimeEq;

use crate::db::users::Role;
use crate::spt::profiles::{list_profiles, SptProfile};
use crate::web::auth::{hash_password, set_session_user, verify_password, SessionUser};
use crate::web::error::WebError;
use crate::web::state::AppState;

// -- Templates --

#[derive(Template)]
#[template(path = "login.html")]
struct LoginTemplate {
    error: Option<String>,
    csrf_token: String,
    flash: Option<crate::web::flash::FlashMessage>,
}

#[derive(Template)]
#[template(path = "register.html")]
struct RegisterTemplate {
    error: Option<String>,
    code: String,
    profiles: Vec<SptProfile>,
    csrf_token: String,
}

// -- Form structs --

#[derive(serde::Deserialize)]
pub struct LoginForm {
    username: String,
    password: String,
    csrf_token: String,
}

#[derive(serde::Deserialize)]
pub struct RegisterForm {
    code: String,
    profile_id: String,
    password: String,
    password_confirm: String,
    csrf_token: String,
}

#[derive(serde::Deserialize)]
pub struct RegisterQuery {
    code: Option<String>,
}

const MIN_PASSWORD_LEN: usize = 8;
const MAX_PASSWORD_LEN: usize = 128;

// -- Helpers --

fn render_register_error(
    msg: &str,
    code: String,
    profiles: Vec<SptProfile>,
    csrf_token: String,
) -> actix_web::Result<HttpResponse> {
    let tmpl = RegisterTemplate {
        error: Some(msg.to_string()),
        code,
        profiles,
        csrf_token,
    };
    Ok(HttpResponse::BadRequest()
        .content_type("text/html")
        .body(tmpl.render().map_err(WebError::from)?))
}

fn is_invite_expired(expires_at: Option<&str>) -> bool {
    let Some(exp) = expires_at else {
        return false;
    };
    match chrono::DateTime::parse_from_rfc3339(exp) {
        Ok(dt) => dt < chrono::Utc::now(),
        Err(_) => exp < chrono::Utc::now().to_rfc3339().as_str(),
    }
}

// -- Handlers --

pub async fn login_page(session: Session) -> actix_web::Result<HttpResponse> {
    let csrf_token = crate::web::csrf::get_or_create_token(&session);
    let flash = crate::web::flash::take_flash(&session);
    let tmpl = LoginTemplate {
        error: None,
        csrf_token,
        flash,
    };
    Ok(HttpResponse::Ok()
        .content_type("text/html")
        .body(tmpl.render().map_err(WebError::from)?))
}

pub async fn login_submit(
    form: Form<LoginForm>,
    state: Data<AppState>,
    session: Session,
) -> actix_web::Result<HttpResponse> {
    let form = form.into_inner();

    if !crate::web::csrf::validate_token(&session, &form.csrf_token) {
        return Err(WebError::Forbidden.into());
    }

    let csrf_token = crate::web::csrf::get_or_create_token(&session);
    let db = state.db.clone();
    let username = form.username.clone();

    let user = web::block(move || {
        let db = db.lock();
        db.get_user_by_username(&username)
    })
    .await
    .map_err(WebError::from)?
    .map_err(WebError::from)?;

    let user = match user {
        Some(u) => u,
        None => {
            let tmpl = LoginTemplate {
                error: Some("Invalid username or password".to_string()),
                csrf_token,
                flash: None,
            };
            return Ok(HttpResponse::Ok()
                .content_type("text/html")
                .body(tmpl.render().map_err(WebError::from)?));
        }
    };

    let valid = match user.password_hash {
        Some(ref hash) => {
            let password = form.password.clone();
            let hash = hash.clone();
            web::block(move || verify_password(&password, &hash))
                .await
                .map_err(WebError::from)?
        }
        None => false,
    };

    if !valid || user.disabled {
        let tmpl = LoginTemplate {
            error: Some("Invalid username or password".to_string()),
            csrf_token,
            flash: None,
        };
        return Ok(HttpResponse::Ok()
            .content_type("text/html")
            .body(tmpl.render().map_err(WebError::from)?));
    }

    session.renew();
    let session_user = SessionUser {
        user_id: user.id,
        username: user.username,
        role: user.role,
    };
    set_session_user(&session, &session_user).map_err(WebError::from)?;

    Ok(HttpResponse::SeeOther()
        .insert_header(("Location", "/quma/"))
        .finish())
}

pub async fn register_page(
    query: Query<RegisterQuery>,
    state: Data<AppState>,
    session: Session,
) -> actix_web::Result<HttpResponse> {
    let csrf_token = crate::web::csrf::get_or_create_token(&session);
    let code = query.code.clone().unwrap_or_default();

    if code.is_empty() {
        let tmpl = RegisterTemplate {
            error: Some("Invite code required".to_string()),
            code: String::new(),
            profiles: vec![],
            csrf_token,
        };
        return Ok(HttpResponse::BadRequest()
            .content_type("text/html")
            .body(tmpl.render().map_err(WebError::from)?));
    }

    let db = state.db.clone();
    let code_check = code.clone();
    let invite = web::block(move || {
        let db = db.lock();
        db.get_invite(&code_check)
    })
    .await
    .map_err(WebError::from)?
    .map_err(WebError::from)?;

    match invite {
        None => {
            let tmpl = RegisterTemplate {
                error: Some("Invalid invite code".to_string()),
                code,
                profiles: vec![],
                csrf_token,
            };
            Ok(HttpResponse::BadRequest()
                .content_type("text/html")
                .body(tmpl.render().map_err(WebError::from)?))
        }
        Some(inv) if inv.used_by.is_some() => {
            let tmpl = RegisterTemplate {
                error: Some("This invite code has already been used".to_string()),
                code,
                profiles: vec![],
                csrf_token,
            };
            Ok(HttpResponse::BadRequest()
                .content_type("text/html")
                .body(tmpl.render().map_err(WebError::from)?))
        }
        Some(ref inv) if is_invite_expired(inv.expires_at.as_deref()) => {
            let tmpl = RegisterTemplate {
                error: Some("This invite code has expired".to_string()),
                code,
                profiles: vec![],
                csrf_token,
            };
            Ok(HttpResponse::BadRequest()
                .content_type("text/html")
                .body(tmpl.render().map_err(WebError::from)?))
        }
        Some(_) => {
            let spt_dir = state.spt_dir.clone();
            let profiles = web::block(move || list_profiles(&spt_dir))
                .await
                .map_err(WebError::from)?
                .unwrap_or_default();
            let tmpl = RegisterTemplate {
                error: None,
                code,
                profiles,
                csrf_token,
            };
            Ok(HttpResponse::Ok()
                .content_type("text/html")
                .body(tmpl.render().map_err(WebError::from)?))
        }
    }
}

pub async fn register_submit(
    form: Form<RegisterForm>,
    state: Data<AppState>,
    session: Session,
) -> actix_web::Result<HttpResponse> {
    let form = form.into_inner();

    if !crate::web::csrf::validate_token(&session, &form.csrf_token) {
        return Err(WebError::Forbidden.into());
    }

    let csrf_token = crate::web::csrf::get_or_create_token(&session);

    let spt_dir = state.spt_dir.clone();
    let profiles = web::block(move || list_profiles(&spt_dir))
        .await
        .map_err(WebError::from)?
        .unwrap_or_default();

    if form.password.len() < MIN_PASSWORD_LEN {
        return render_register_error(
            &format!("Password must be at least {MIN_PASSWORD_LEN} characters"),
            form.code,
            profiles,
            csrf_token,
        );
    }

    if form.password.len() > MAX_PASSWORD_LEN {
        return render_register_error(
            &format!("Password must be at most {MAX_PASSWORD_LEN} characters"),
            form.code,
            profiles,
            csrf_token,
        );
    }

    if form.password != form.password_confirm {
        return render_register_error("Passwords do not match", form.code, profiles, csrf_token);
    }

    if form.profile_id.is_empty() {
        return render_register_error(
            "Please select your SPT profile",
            form.code,
            profiles,
            csrf_token,
        );
    }

    let profile = profiles.iter().find(|p| p.aid == form.profile_id);

    let username = match profile {
        Some(p) => p.username.clone(),
        None => {
            return render_register_error(
                "Invalid profile selection",
                form.code,
                profiles,
                csrf_token,
            );
        }
    };

    let password = form.password.clone();
    let password_hash = web::block(move || hash_password(&password))
        .await
        .map_err(WebError::from)?
        .map_err(WebError::from)?;

    let db = state.db.clone();
    let code = form.code.clone();
    let profile_id = form.profile_id.clone();

    let result = web::block(move || {
        let db = db.lock();

        if db.get_user_by_username(&username)?.is_some() {
            return Ok::<_, rusqlite::Error>(Err(
                "A user with this profile already exists".to_string()
            ));
        }

        // Consume the invite atomically (prevents race with concurrent registrations)
        let used = db.use_invite(&code, 0)?;
        if used == 0 {
            return Ok(Err("Invite code is invalid or expired".to_string()));
        }

        let user_id = db.insert_user(
            &username,
            Some(&profile_id),
            Some(&password_hash),
            Role::Player,
        )?;

        // Update the invite to point to the real user_id (no IS NULL guard needed)
        db.update_invite_user(&code, user_id)?;

        Ok(Ok(user_id))
    })
    .await
    .map_err(WebError::from)?
    .map_err(WebError::from)?;

    match result {
        Ok(_user_id) => Ok(HttpResponse::SeeOther()
            .insert_header(("Location", "/quma/login"))
            .finish()),
        Err(msg) => render_register_error(&msg, form.code, profiles, csrf_token),
    }
}

pub async fn logout(session: Session, form: Form<crate::web::csrf::CsrfForm>) -> HttpResponse {
    if !crate::web::csrf::validate_token(&session, &form.csrf_token) {
        return HttpResponse::Forbidden().body("forbidden");
    }
    session.purge();
    HttpResponse::SeeOther()
        .insert_header(("Location", "/login"))
        .finish()
}

// -- Reset password --

#[derive(serde::Deserialize)]
pub struct ResetPasswordQuery {
    token: Option<String>,
}

#[derive(serde::Deserialize)]
pub struct ResetPasswordForm {
    token: String,
    password: String,
    password_confirm: String,
    csrf_token: String,
}

#[derive(Template)]
#[template(path = "reset_password.html")]
struct ResetPasswordTemplate {
    error: Option<String>,
    token: String,
    token_valid: bool,
    csrf_token: String,
    flash: Option<crate::web::flash::FlashMessage>,
}

fn is_token_expired(expires_at: &str) -> bool {
    match chrono::DateTime::parse_from_rfc3339(expires_at) {
        Ok(dt) => dt < chrono::Utc::now(),
        Err(_) => expires_at < chrono::Utc::now().to_rfc3339().as_str(),
    }
}

pub async fn reset_password_page(
    query: Query<ResetPasswordQuery>,
    state: Data<AppState>,
    session: Session,
) -> actix_web::Result<HttpResponse> {
    let csrf_token = crate::web::csrf::get_or_create_token(&session);
    let token = query.token.clone().unwrap_or_default();

    if token.is_empty() {
        let tmpl = ResetPasswordTemplate {
            error: Some("This password reset link is invalid or has already been used. Please contact an administrator for a new link.".to_string()),
            token: String::new(),
            token_valid: false,
            csrf_token,
            flash: None,
        };
        return Ok(HttpResponse::BadRequest()
            .content_type("text/html")
            .body(tmpl.render().map_err(WebError::from)?));
    }

    let db = state.db.clone();
    let token_clone = token.clone();
    let reset_token = web::block(move || {
        let db = db.lock();
        db.get_reset_token(&token_clone)
    })
    .await
    .map_err(WebError::from)?
    .map_err(WebError::from)?;

    let valid = match reset_token {
        Some(ref rt) => {
            let ct_match = token.as_bytes().ct_eq(rt.token.as_bytes());
            bool::from(ct_match) && !is_token_expired(&rt.expires_at)
        }
        None => false,
    };

    if !valid {
        let tmpl = ResetPasswordTemplate {
            error: Some("This password reset link is invalid or has already been used. Please contact an administrator for a new link.".to_string()),
            token: String::new(),
            token_valid: false,
            csrf_token,
            flash: None,
        };
        return Ok(HttpResponse::BadRequest()
            .content_type("text/html")
            .body(tmpl.render().map_err(WebError::from)?));
    }

    // Check user exists and is not disabled
    let reset_token = reset_token.unwrap();
    let db2 = state.db.clone();
    let uid = reset_token.user_id;
    let target_user = web::block(move || {
        let db = db2.lock();
        db.get_user_by_id(uid)
    })
    .await
    .map_err(WebError::from)?
    .map_err(WebError::from)?;

    match target_user {
        Some(u) if !u.disabled => {} // OK
        _ => {
            let tmpl = ResetPasswordTemplate {
                error: Some("This password reset link is invalid or has already been used. Please contact an administrator for a new link.".to_string()),
                token: String::new(),
                token_valid: false,
                csrf_token,
                flash: None,
            };
            return Ok(HttpResponse::BadRequest()
                .content_type("text/html")
                .body(tmpl.render().map_err(WebError::from)?));
        }
    }

    let tmpl = ResetPasswordTemplate {
        error: None,
        token,
        token_valid: true,
        csrf_token,
        flash: None,
    };
    Ok(HttpResponse::Ok()
        .content_type("text/html")
        .body(tmpl.render().map_err(WebError::from)?))
}

pub async fn reset_password_submit(
    form: Form<ResetPasswordForm>,
    state: Data<AppState>,
    session: Session,
) -> actix_web::Result<HttpResponse> {
    let form = form.into_inner();

    if !crate::web::csrf::validate_token(&session, &form.csrf_token) {
        return Err(WebError::Forbidden.into());
    }

    let csrf_token = crate::web::csrf::get_or_create_token(&session);
    let render_error = |msg: &str, token: String| -> actix_web::Result<HttpResponse> {
        let tmpl = ResetPasswordTemplate {
            error: Some(msg.to_string()),
            token,
            token_valid: true,
            csrf_token: csrf_token.clone(),
            flash: None,
        };
        Ok(HttpResponse::BadRequest()
            .content_type("text/html")
            .body(tmpl.render().map_err(WebError::from)?))
    };

    if form.password.len() < MIN_PASSWORD_LEN {
        return render_error("Password must be 8-128 characters", form.token);
    }
    if form.password.len() > MAX_PASSWORD_LEN {
        return render_error("Password must be 8-128 characters", form.token);
    }
    if form.password != form.password_confirm {
        return render_error("Passwords do not match", form.token);
    }

    let db = state.db.clone();
    let token_clone = form.token.clone();
    let reset_token = web::block(move || {
        let db = db.lock();
        db.get_reset_token(&token_clone)
    })
    .await
    .map_err(WebError::from)?
    .map_err(WebError::from)?;

    let rt = match reset_token {
        Some(rt) => {
            let ct_match = form.token.as_bytes().ct_eq(rt.token.as_bytes());
            if !bool::from(ct_match) || is_token_expired(&rt.expires_at) {
                return render_error(
                    "This password reset link is invalid or has already been used. Please contact an administrator for a new link.",
                    String::new(),
                );
            }
            rt
        }
        None => {
            return render_error(
                "This password reset link is invalid or has already been used. Please contact an administrator for a new link.",
                String::new(),
            );
        }
    };

    // Check user exists and is not disabled
    let db2 = state.db.clone();
    let uid = rt.user_id;
    let target_user = web::block(move || {
        let db = db2.lock();
        db.get_user_by_id(uid)
    })
    .await
    .map_err(WebError::from)?
    .map_err(WebError::from)?;

    match target_user {
        Some(u) if !u.disabled => {} // OK
        _ => {
            return render_error(
                "This password reset link is invalid or has already been used. Please contact an administrator for a new link.",
                String::new(),
            );
        }
    }

    let password = form.password.clone();
    let password_hash = web::block(move || hash_password(&password))
        .await
        .map_err(WebError::from)?
        .map_err(WebError::from)?;

    let db = state.db.clone();
    let user_id = rt.user_id;
    let token_to_delete = rt.token.clone();
    web::block(move || {
        let db = db.lock();
        db.update_user_password(user_id, &password_hash)?;
        db.delete_reset_token(&token_to_delete)?;
        Ok::<_, rusqlite::Error>(())
    })
    .await
    .map_err(WebError::from)?
    .map_err(WebError::from)?;

    crate::web::flash::set_flash(&session, "Password updated — please log in.", "success");

    Ok(HttpResponse::SeeOther()
        .insert_header(("Location", "/login"))
        .finish())
}
