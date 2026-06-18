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

    // Argon2 verification is CPU-intensive — run on blocking thread pool
    let valid = match user.password_hash.clone() {
        Some(hash) => {
            let password = form.password.clone();
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
        Some(ref inv)
            if inv
                .expires_at
                .as_deref()
                .is_some_and(|exp| exp < chrono::Utc::now().to_rfc3339().as_str()) =>
        {
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
            let profiles = list_profiles(&state.spt_dir).unwrap_or_default();
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
    session: Session,
) -> actix_web::Result<HttpResponse> {
    let form = form.into_inner();

    if form.password != form.password_confirm {
        let profiles = list_profiles(&state.spt_dir).unwrap_or_default();
        let tmpl = RegisterTemplate {
            error: Some("Passwords do not match".to_string()),
            code: form.code,
            profiles,
        };
        return Ok(HttpResponse::Ok()
            .content_type("text/html")
            .body(tmpl.render().map_err(WebError::from)?));
    }

    if form.profile_id.is_empty() {
        let profiles = list_profiles(&state.spt_dir).unwrap_or_default();
        let tmpl = RegisterTemplate {
            error: Some("Please select your SPT profile".to_string()),
            code: form.code,
            profiles,
        };
        return Ok(HttpResponse::Ok()
            .content_type("text/html")
            .body(tmpl.render().map_err(WebError::from)?));
    }

    let profiles = list_profiles(&state.spt_dir).unwrap_or_default();
    let profile = profiles.iter().find(|p| p.aid == form.profile_id);

    let username = match profile {
        Some(p) => p.username.clone(),
        None => {
            let tmpl = RegisterTemplate {
                error: Some("Invalid profile selection".to_string()),
                code: form.code,
                profiles,
            };
            return Ok(HttpResponse::Ok()
                .content_type("text/html")
                .body(tmpl.render().map_err(WebError::from)?));
        }
    };

    // Argon2 hashing is CPU-intensive — run on blocking thread pool
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

        let user_id = db.insert_user(&username, &profile_id, Some(&password_hash), "player")?;
        let used = db.use_invite(&code, user_id)?;
        if used == 0 {
            return Ok(Err("Invite code is invalid or expired".to_string()));
        }

        Ok(Ok(user_id))
    })
    .await
    .map_err(WebError::from)?
    .map_err(WebError::from)?;

    match result {
        Ok(_user_id) => Ok(HttpResponse::SeeOther()
            .insert_header(("Location", "/login"))
            .finish()),
        Err(msg) => {
            let profiles = list_profiles(&state.spt_dir).unwrap_or_default();
            let tmpl = RegisterTemplate {
                error: Some(msg),
                code: form.code,
                profiles,
            };
            Ok(HttpResponse::Ok()
                .content_type("text/html")
                .body(tmpl.render().map_err(WebError::from)?))
        }
    }
}

pub async fn logout(session: Session) -> HttpResponse {
    session.purge();
    HttpResponse::SeeOther()
        .insert_header(("Location", "/login"))
        .finish()
}
