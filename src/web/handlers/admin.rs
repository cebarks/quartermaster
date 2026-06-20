use actix_session::Session;
use actix_web::web::{self, Data, Form, Html, Path};
use actix_web::{HttpMessage, HttpRequest};
use askama::Template;

use crate::db::users::{InviteCodeWithUsers, Role, User};
use crate::spt::profiles::{load_all_profile_stats, ProfileStatus, SptProfileStats};
use crate::web::auth::{require_auth, SessionUser};
use crate::web::error::WebError;
use crate::web::state::AppState;

fn is_invite_expired(expires_at: Option<&str>) -> bool {
    let Some(exp) = expires_at else {
        return false;
    };
    match chrono::DateTime::parse_from_rfc3339(exp) {
        Ok(dt) => dt < chrono::Utc::now(),
        Err(_) => exp < chrono::Utc::now().to_rfc3339().as_str(),
    }
}

fn build_user_profiles(
    users: &[User],
    spt_dir: &std::path::Path,
    profile_stats: &std::collections::HashMap<String, SptProfileStats>,
) -> Vec<ProfileStatus> {
    users
        .iter()
        .map(|u| {
            if u.spt_profile_id.is_empty() {
                return ProfileStatus::NotFound;
            }
            match profile_stats.get(&u.spt_profile_id) {
                Some(stats) => ProfileStatus::Found(stats.clone()),
                None => {
                    let profile_path = spt_dir
                        .join("SPT/user/profiles")
                        .join(format!("{}.json", u.spt_profile_id));
                    if profile_path.exists() {
                        ProfileStatus::ParseError
                    } else {
                        ProfileStatus::NotFound
                    }
                }
            }
        })
        .collect()
}

// -- Templates --

#[derive(Template)]

#[template(path = "admin.html")]
struct AdminPageTemplate {
    user: SessionUser,
    csrf_token: String,
    users: Vec<(User, ProfileStatus)>,
    current_user_id: i64,
    flash: Option<crate::web::flash::FlashMessage>,
    #[allow(dead_code)]
    fika_installed: bool,
    #[allow(dead_code)]
    modsync_installed: bool,
}

#[derive(Template)]
#[template(path = "admin/partials/users.html")]
struct UsersPartialTemplate {
    users: Vec<(User, ProfileStatus)>,
    current_user_id: i64,
    csrf_token: String,
}

#[derive(Template)]
#[template(path = "admin/partials/user_row.html")]
struct UserRowTemplate {
    u: User,
    profile: ProfileStatus,
    current_user_id: i64,
    csrf_token: String,
    reset_link: Option<String>,
    row_message: Option<String>,
}

// InviteView -- pre-computed view struct for invites template
// (Askama can't call free functions, so we pre-compute status)
pub struct InviteView {
    pub code: String,
    pub created_by_username: Option<String>,
    pub used_by_username: Option<String>,
    pub created_at: String,
    pub expires_at: Option<String>,
    pub status: String, // "available", "used", or "expired"
}

impl InviteView {
    fn from_db(ic: InviteCodeWithUsers) -> Self {
        let status = if ic.invite.used_by.is_some() {
            "used"
        } else if is_invite_expired(ic.invite.expires_at.as_deref()) {
            "expired"
        } else {
            "available"
        };
        InviteView {
            code: ic.invite.code,
            created_by_username: ic.created_by_username,
            used_by_username: ic.used_by_username,
            created_at: ic.invite.created_at,
            expires_at: ic.invite.expires_at,
            status: status.to_string(),
        }
    }
}

#[derive(Template)]
#[template(path = "admin/partials/invites.html")]
struct InvitesPartialTemplate {
    invites: Vec<InviteView>,
    csrf_token: String,
    highlight_code: Option<String>,
}

// -- Form structs --

#[derive(serde::Deserialize)]
pub struct RoleForm {
    role: String,
    csrf_token: String,
}

#[derive(serde::Deserialize)]
pub struct CsrfOnly {
    csrf_token: String,
}

#[derive(serde::Deserialize)]
pub struct InviteForm {
    expiry: String,
    csrf_token: String,
}

// -- Admin scope middleware --

pub async fn admin_middleware(
    req: actix_web::dev::ServiceRequest,
    next: actix_web::middleware::Next<actix_web::body::BoxBody>,
) -> Result<actix_web::dev::ServiceResponse<actix_web::body::BoxBody>, actix_web::Error> {
    let user = req
        .extensions()
        .get::<SessionUser>()
        .cloned()
        .ok_or(WebError::Forbidden)?;

    if !user.role.can_manage_users() {
        return Err(WebError::Forbidden.into());
    }

    next.call(req).await
}

// -- Handlers --

pub async fn admin_page(
    state: Data<AppState>,
    req: HttpRequest,
    session: Session,
) -> actix_web::Result<Html> {
    let user = require_auth(&req)?;
    crate::web::auth::require_capability(&user, Role::can_manage_users)?;
    let csrf_token = crate::web::csrf::get_or_create_token(&session);
    let flash = crate::web::flash::take_flash(&session);

    let db = state.db.clone();
    let all_users = web::block(move || {
        let db = db.lock();
        db.list_users()
    })
    .await
    .map_err(WebError::from)?
    .map_err(WebError::from)?;

    let spt_dir = state.spt_dir.clone();
    let profile_stats = web::block(move || load_all_profile_stats(&spt_dir))
        .await
        .map_err(WebError::from)?;

    let profiles = build_user_profiles(&all_users, &state.spt_dir, &profile_stats);
    let users: Vec<(User, ProfileStatus)> = all_users.into_iter().zip(profiles).collect();
    let current_user_id = user.user_id;

    let tmpl = AdminPageTemplate {
        user,
        csrf_token,
        users,
        current_user_id,
        flash,
        fika_installed: state.fika_installed,
        modsync_installed: state.is_modsync_installed(),
    };
    Ok(Html::new(tmpl.render().map_err(WebError::from)?))
}

pub async fn admin_users(
    state: Data<AppState>,
    req: HttpRequest,
    session: Session,
) -> actix_web::Result<Html> {
    let user = require_auth(&req)?;
    let csrf_token = crate::web::csrf::get_or_create_token(&session);

    let db = state.db.clone();
    let all_users = web::block(move || {
        let db = db.lock();
        db.list_users()
    })
    .await
    .map_err(WebError::from)?
    .map_err(WebError::from)?;

    let spt_dir = state.spt_dir.clone();
    let profile_stats = web::block(move || load_all_profile_stats(&spt_dir))
        .await
        .map_err(WebError::from)?;

    let profiles = build_user_profiles(&all_users, &state.spt_dir, &profile_stats);
    let users: Vec<(User, ProfileStatus)> = all_users.into_iter().zip(profiles).collect();

    let tmpl = UsersPartialTemplate {
        users,
        current_user_id: user.user_id,
        csrf_token,
    };
    Ok(Html::new(tmpl.render().map_err(WebError::from)?))
}

pub async fn change_role(
    state: Data<AppState>,
    req: HttpRequest,
    session: Session,
    path: Path<i64>,
    form: Form<RoleForm>,
) -> actix_web::Result<Html> {
    let current_user = require_auth(&req)?;
    if !crate::web::csrf::validate_token(&session, &form.csrf_token) {
        return Err(WebError::Forbidden.into());
    }

    let target_id = path.into_inner();
    if target_id == current_user.user_id {
        return Err(WebError::Forbidden.into());
    }

    let new_role = Role::try_from(form.role.clone())
        .map_err(|_| WebError::BadRequest("Invalid role".to_string()))?;

    let db = state.db.clone();
    let affected = web::block(move || {
        let db = db.lock();
        db.update_user_role(target_id, new_role)
    })
    .await
    .map_err(WebError::from)?
    .map_err(WebError::from)?;

    if affected == 0 {
        return Err(
            WebError::UnprocessableEntity("Cannot demote the last admin".to_string()).into(),
        );
    }

    render_user_row(
        &state,
        &session,
        target_id,
        current_user.user_id,
        None,
        None,
    )
    .await
}

pub async fn toggle_disable(
    state: Data<AppState>,
    req: HttpRequest,
    session: Session,
    path: Path<i64>,
    form: Form<CsrfOnly>,
) -> actix_web::Result<Html> {
    let current_user = require_auth(&req)?;
    if !crate::web::csrf::validate_token(&session, &form.csrf_token) {
        return Err(WebError::Forbidden.into());
    }

    let target_id = path.into_inner();
    if target_id == current_user.user_id {
        return Err(WebError::Forbidden.into());
    }

    let db = state.db.clone();
    let target_user = web::block(move || {
        let db = db.lock();
        db.get_user_by_id(target_id)
    })
    .await
    .map_err(WebError::from)?
    .map_err(WebError::from)?
    .ok_or(WebError::NotFound)?;

    let new_disabled = !target_user.disabled;
    let db = state.db.clone();
    let affected = web::block(move || {
        let db = db.lock();
        db.set_user_disabled(target_id, new_disabled)
    })
    .await
    .map_err(WebError::from)?
    .map_err(WebError::from)?;

    if affected == 0 {
        return Err(
            WebError::UnprocessableEntity("Cannot disable the last admin".to_string()).into(),
        );
    }

    let message = if new_disabled {
        Some("User disabled — will be logged out on their next request.".to_string())
    } else {
        None
    };

    render_user_row(
        &state,
        &session,
        target_id,
        current_user.user_id,
        None,
        message,
    )
    .await
}

pub async fn create_reset_token(
    state: Data<AppState>,
    req: HttpRequest,
    session: Session,
    path: Path<i64>,
    form: Form<CsrfOnly>,
) -> actix_web::Result<Html> {
    let current_user = require_auth(&req)?;
    if !crate::web::csrf::validate_token(&session, &form.csrf_token) {
        return Err(WebError::Forbidden.into());
    }

    let target_id = path.into_inner();

    // Verify user exists
    let db = state.db.clone();
    let target_user = web::block(move || {
        let db = db.lock();
        db.get_user_by_id(target_id)
    })
    .await
    .map_err(WebError::from)?
    .map_err(WebError::from)?
    .ok_or(WebError::NotFound)?;

    if target_user.disabled {
        return Err(
            WebError::BadRequest("Cannot reset password for a disabled user".to_string()).into(),
        );
    }

    // Generate CSPRNG token via thread-local CSPRNG (ChaCha-based in rand 0.9)
    use rand::Rng;
    let mut token_bytes = [0u8; 32];
    rand::rng().fill_bytes(&mut token_bytes);
    let token = base64_url_encode(&token_bytes);

    let expires_at = (chrono::Utc::now() + chrono::Duration::hours(24)).to_rfc3339();

    let db = state.db.clone();
    let token_clone = token.clone();
    web::block(move || {
        let db = db.lock();
        db.create_reset_token(target_id, &token_clone, &expires_at)
    })
    .await
    .map_err(WebError::from)?
    .map_err(WebError::from)?;

    let host = req.connection_info().host().to_string();
    let scheme = req.connection_info().scheme().to_string();
    let reset_link = format!("{scheme}://{host}/reset-password?token={token}");
    render_user_row(
        &state,
        &session,
        target_id,
        current_user.user_id,
        Some(reset_link),
        None,
    )
    .await
}

pub async fn admin_invites(
    state: Data<AppState>,
    req: HttpRequest,
    session: Session,
) -> actix_web::Result<Html> {
    require_auth(&req)?;
    let csrf_token = crate::web::csrf::get_or_create_token(&session);

    let db = state.db.clone();
    let db_invites = web::block(move || {
        let db = db.lock();
        db.list_invite_codes()
    })
    .await
    .map_err(WebError::from)?
    .map_err(WebError::from)?;

    let invites: Vec<InviteView> = db_invites.into_iter().map(InviteView::from_db).collect();

    let tmpl = InvitesPartialTemplate {
        invites,
        csrf_token,
        highlight_code: None,
    };
    Ok(Html::new(tmpl.render().map_err(WebError::from)?))
}

pub async fn create_invite(
    state: Data<AppState>,
    req: HttpRequest,
    session: Session,
    form: Form<InviteForm>,
) -> actix_web::Result<Html> {
    let current_user = require_auth(&req)?;
    if !crate::web::csrf::validate_token(&session, &form.csrf_token) {
        return Err(WebError::Forbidden.into());
    }

    let code = crate::invite::generate_invite_code();

    let expires_at = if form.expiry == "never" {
        None
    } else {
        Some(
            crate::invite::parse_expiry(&form.expiry)
                .map_err(|_| WebError::BadRequest("Invalid expiry value".to_string()))?,
        )
    };

    let db = state.db.clone();
    let code_clone = code.clone();
    let user_id = current_user.user_id;
    web::block(move || {
        let db = db.lock();
        db.create_invite(&code_clone, Some(user_id), expires_at.as_deref())
    })
    .await
    .map_err(WebError::from)?
    .map_err(WebError::from)?;

    let csrf_token = crate::web::csrf::get_or_create_token(&session);
    let db = state.db.clone();
    let db_invites = web::block(move || {
        let db = db.lock();
        db.list_invite_codes()
    })
    .await
    .map_err(WebError::from)?
    .map_err(WebError::from)?;

    let invites: Vec<InviteView> = db_invites.into_iter().map(InviteView::from_db).collect();

    let tmpl = InvitesPartialTemplate {
        invites,
        csrf_token,
        highlight_code: Some(code),
    };
    Ok(Html::new(tmpl.render().map_err(WebError::from)?))
}

// -- Helpers --

async fn render_user_row(
    state: &Data<AppState>,
    session: &Session,
    user_id: i64,
    current_user_id: i64,
    reset_link: Option<String>,
    row_message: Option<String>,
) -> actix_web::Result<Html> {
    let csrf_token = crate::web::csrf::get_or_create_token(session);

    let db = state.db.clone();
    let user = web::block(move || {
        let db = db.lock();
        db.get_user_by_id(user_id)
    })
    .await
    .map_err(WebError::from)?
    .map_err(WebError::from)?
    .ok_or(WebError::NotFound)?;

    let spt_dir = state.spt_dir.clone();
    let aid = user.spt_profile_id.clone();
    let profile_stats = web::block(move || load_all_profile_stats(&spt_dir))
        .await
        .map_err(WebError::from)?;

    let profile = if aid.is_empty() {
        ProfileStatus::NotFound
    } else {
        match profile_stats.get(&aid) {
            Some(stats) => ProfileStatus::Found(stats.clone()),
            None => {
                let path = state
                    .spt_dir
                    .join("SPT/user/profiles")
                    .join(format!("{aid}.json"));
                if path.exists() {
                    ProfileStatus::ParseError
                } else {
                    ProfileStatus::NotFound
                }
            }
        }
    };

    let tmpl = UserRowTemplate {
        u: user,
        profile,
        current_user_id,
        csrf_token,
        reset_link,
        row_message,
    };
    Ok(Html::new(tmpl.render().map_err(WebError::from)?))
}

fn base64_url_encode(bytes: &[u8]) -> String {
    const CHARS: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789-_";
    let mut result = String::new();
    for chunk in bytes.chunks(3) {
        let b0 = chunk[0] as u32;
        let b1 = chunk.get(1).copied().unwrap_or(0) as u32;
        let b2 = chunk.get(2).copied().unwrap_or(0) as u32;
        let n = (b0 << 16) | (b1 << 8) | b2;
        result.push(CHARS[((n >> 18) & 0x3F) as usize] as char);
        result.push(CHARS[((n >> 12) & 0x3F) as usize] as char);
        if chunk.len() > 1 {
            result.push(CHARS[((n >> 6) & 0x3F) as usize] as char);
        }
        if chunk.len() > 2 {
            result.push(CHARS[(n & 0x3F) as usize] as char);
        }
    }
    result
}
