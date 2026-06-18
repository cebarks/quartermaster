use actix_session::Session;
use actix_web::web::{self, Data, Form, Query};
use actix_web::HttpResponse;
use askama::Template;

use crate::spt::profiles::{list_profiles, SptProfile};
use crate::web::auth::{hash_password, set_session_user, verify_password, SessionUser};
use crate::web::error::WebError;
use crate::web::state::AppState;

// -- Templates --

#[derive(Template)]
#[template(path = "login.html")]
struct LoginTemplate {
    error: Option<String>,
}

#[derive(Template)]
#[template(path = "register.html")]
struct RegisterTemplate {
    error: Option<String>,
    code: String,
    profiles: Vec<SptProfile>,
}

// -- Form structs --

#[derive(serde::Deserialize)]
pub struct LoginForm {
    username: String,
    password: String,
}

#[derive(serde::Deserialize)]
pub struct RegisterForm {
    code: String,
    profile_id: String,
    password: String,
    password_confirm: String,
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
) -> actix_web::Result<HttpResponse> {
    let tmpl = RegisterTemplate {
        error: Some(msg.to_string()),
        code,
        profiles,
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

pub async fn login_page() -> actix_web::Result<HttpResponse> {
    let tmpl = LoginTemplate { error: None };
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

    if !valid {
        let tmpl = LoginTemplate {
            error: Some("Invalid username or password".to_string()),
        };
        return Ok(HttpResponse::Ok()
            .content_type("text/html")
            .body(tmpl.render().map_err(WebError::from)?));
    }

    let session_user = SessionUser {
        user_id: user.id,
        username: user.username,
        role: user.role,
    };
    set_session_user(&session, &session_user).map_err(WebError::from)?;

    Ok(HttpResponse::SeeOther()
        .insert_header(("Location", "/"))
        .finish())
}

pub async fn register_page(
    query: Query<RegisterQuery>,
    state: Data<AppState>,
) -> actix_web::Result<HttpResponse> {
    let code = query.code.clone().unwrap_or_default();

    if code.is_empty() {
        let tmpl = RegisterTemplate {
            error: Some("Invite code required".to_string()),
            code: String::new(),
            profiles: vec![],
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
    _session: Session,
) -> actix_web::Result<HttpResponse> {
    let form = form.into_inner();

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
        );
    }

    if form.password.len() > MAX_PASSWORD_LEN {
        return render_register_error(
            &format!("Password must be at most {MAX_PASSWORD_LEN} characters"),
            form.code,
            profiles,
        );
    }

    if form.password != form.password_confirm {
        return render_register_error("Passwords do not match", form.code, profiles);
    }

    if form.profile_id.is_empty() {
        return render_register_error("Please select your SPT profile", form.code, profiles);
    }

    let profile = profiles.iter().find(|p| p.aid == form.profile_id);

    let username = match profile {
        Some(p) => p.username.clone(),
        None => {
            return render_register_error("Invalid profile selection", form.code, profiles);
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

        // Consume the invite BEFORE creating the user to prevent orphaned accounts
        // when concurrent requests race on the same invite code
        let used = db.use_invite(&code, 0)?;
        if used == 0 {
            return Ok(Err("Invite code is invalid or expired".to_string()));
        }

        let user_id = db.insert_user(&username, &profile_id, Some(&password_hash), "player")?;

        // Update the invite to point to the real user_id
        db.use_invite(&code, user_id).ok();

        Ok(Ok(user_id))
    })
    .await
    .map_err(WebError::from)?
    .map_err(WebError::from)?;

    match result {
        Ok(_user_id) => Ok(HttpResponse::SeeOther()
            .insert_header(("Location", "/login"))
            .finish()),
        Err(msg) => render_register_error(&msg, form.code, profiles),
    }
}

pub async fn logout(session: Session) -> HttpResponse {
    session.purge();
    HttpResponse::SeeOther()
        .insert_header(("Location", "/login"))
        .finish()
}
