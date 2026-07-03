use actix_session::Session;
use actix_web::web::{self, Data, Form, Query};
use actix_web::HttpResponse;
use askama::Template;
use subtle::ConstantTimeEq;

use actix_web::HttpRequest;

use crate::web::auth::{
    hash_password, require_auth, set_session_user, validate_password_complexity, verify_password,
    SessionUser,
};
use crate::web::error::WebError;
use crate::web::nav::NavContext;
use crate::web::state::AppState;

// -- Templates --

#[derive(Template)]
#[template(path = "login.html")]
struct LoginTemplate {
    error: Option<String>,
    csrf_token: String,
    flash: Option<crate::web::flash::FlashMessage>,
}

// -- Form structs --

#[derive(serde::Deserialize)]
pub struct LoginForm {
    username: String,
    password: String,
    csrf_token: String,
}

// -- Helpers --

// -- Handlers --

#[derive(serde::Deserialize)]
pub struct LoginQuery {
    pw: Option<String>,
}

pub async fn login_page(
    session: Session,
    query: Query<LoginQuery>,
) -> actix_web::Result<HttpResponse> {
    let csrf_token = crate::web::csrf::get_or_create_token(&session);
    let mut flash = crate::web::flash::take_flash(&session);
    if flash.is_none() && query.pw.as_deref() == Some("changed") {
        flash = Some(crate::web::flash::FlashMessage {
            message: "Password updated — please log in.".to_string(),
            flash_type: crate::web::flash::FlashType::Success,
        });
    }
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

    // Load permissions for session setup (middleware handles subsequent requests)
    let db = state.db.clone();
    let role = user.role.clone();
    let (role_display, permissions) = web::block(move || {
        let db = db.lock();
        let role_display = db
            .get_role_by_name(&role)
            .ok()
            .flatten()
            .map(|r| r.display_name)
            .unwrap_or_else(|| role.clone());
        let permissions = db.get_permissions_for_role(&role).unwrap_or_default();
        (role_display, permissions)
    })
    .await
    .map_err(WebError::from)?;

    session.renew();
    let session_user = SessionUser {
        user_id: user.id,
        has_password: user.password_hash.is_some(),
        username: user.username,
        role_name: user.role,
        role_display_name: role_display,
        permissions,
    };
    set_session_user(&session, &session_user).map_err(WebError::from)?;

    Ok(HttpResponse::SeeOther()
        .insert_header(("Location", "/quma/"))
        .finish())
}

pub async fn logout(session: Session, form: Form<crate::web::csrf::CsrfForm>) -> HttpResponse {
    if !crate::web::csrf::validate_token(&session, &form.csrf_token) {
        return HttpResponse::Forbidden().body("forbidden");
    }
    session.purge();
    HttpResponse::SeeOther()
        .insert_header(("Location", "/quma/login"))
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
    let reset_token = reset_token.expect("None case returned above");
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

    if let Err(msg) = validate_password_complexity(&form.password) {
        return render_error(msg, form.token);
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

    crate::web::flash::set_flash(
        &session,
        "Password updated — please log in.",
        crate::web::flash::FlashType::Success,
    );

    Ok(HttpResponse::SeeOther()
        .insert_header(("Location", "/quma/login"))
        .finish())
}

// -- Change password (self-service) --

#[derive(serde::Deserialize)]
pub struct ChangePasswordForm {
    current_password: String,
    password: String,
    password_confirm: String,
    csrf_token: String,
}

#[derive(Template)]
#[template(path = "change_password.html")]
struct ChangePasswordTemplate {
    error: Option<String>,
    csrf_token: String,
    user: SessionUser,
    nav: NavContext,
    flash: Option<crate::web::flash::FlashMessage>,
}

pub async fn change_password_page(
    req: HttpRequest,
    session: Session,
    state: Data<AppState>,
) -> actix_web::Result<HttpResponse> {
    let user = require_auth(&req)?;
    let csrf_token = crate::web::csrf::get_or_create_token(&session);
    let nav = NavContext::from_state(&state);
    let tmpl = ChangePasswordTemplate {
        error: None,
        csrf_token,
        user,
        nav,
        flash: None,
    };
    Ok(HttpResponse::Ok()
        .content_type("text/html")
        .body(tmpl.render().map_err(WebError::from)?))
}

pub async fn change_password_submit(
    req: HttpRequest,
    form: Form<ChangePasswordForm>,
    state: Data<AppState>,
    session: Session,
) -> actix_web::Result<HttpResponse> {
    let user = require_auth(&req)?;
    let form = form.into_inner();

    if !crate::web::csrf::validate_token(&session, &form.csrf_token) {
        return Err(WebError::Forbidden.into());
    }

    let csrf_token = crate::web::csrf::get_or_create_token(&session);

    let render_error = |msg: &str| -> actix_web::Result<HttpResponse> {
        let tmpl = ChangePasswordTemplate {
            error: Some(msg.to_string()),
            csrf_token: csrf_token.clone(),
            user: user.clone(),
            nav: NavContext::from_state(&state),
            flash: None,
        };
        Ok(HttpResponse::BadRequest()
            .content_type("text/html")
            .body(tmpl.render().map_err(WebError::from)?))
    };

    // Look up the user to get their current password hash
    let db = state.db.clone();
    let user_id = user.user_id;
    let db_user = web::block(move || {
        let db = db.lock();
        db.get_user_by_id(user_id)
    })
    .await
    .map_err(WebError::from)?
    .map_err(WebError::from)?;

    let db_user = match db_user {
        Some(u) => u,
        None => return Err(WebError::Forbidden.into()),
    };

    // Users without a password hash (profile-only) cannot change password
    let existing_hash = match db_user.password_hash {
        Some(h) => h,
        None => {
            return render_error("Your account does not have a password set.");
        }
    };

    // Validate new password before expensive verify/hash operations
    if let Err(msg) = validate_password_complexity(&form.password) {
        return render_error(msg);
    }
    if form.password != form.password_confirm {
        return render_error("New passwords do not match.");
    }

    // Verify current password
    let current_password = form.current_password;
    let current_valid = web::block(move || verify_password(&current_password, &existing_hash))
        .await
        .map_err(WebError::from)?;

    if !current_valid {
        return render_error("Current password is incorrect.");
    }

    // Hash and save
    let new_password = form.password;
    let new_hash = web::block(move || hash_password(&new_password))
        .await
        .map_err(WebError::from)?
        .map_err(WebError::from)?;

    let db = state.db.clone();
    web::block(move || {
        let db = db.lock();
        db.update_user_password(user_id, &new_hash)?;
        Ok::<_, rusqlite::Error>(())
    })
    .await
    .map_err(WebError::from)?
    .map_err(WebError::from)?;

    session.purge();

    Ok(HttpResponse::SeeOther()
        .insert_header(("Location", "/quma/login?pw=changed"))
        .finish())
}
