use actix_session::Session;
use actix_web::web::{self, Data, Form, Html, Path};
use actix_web::{HttpMessage, HttpRequest};
use askama::Template;

use crate::db::rbac::{
    validate_role_name, DeleteRoleResult, Permission, PermissionInfo, RoleRecord,
    RoleWithPermissions,
};
use crate::db::users::{DeleteInviteResult, DeleteUserResult, InviteCodeWithUsers, User};
use crate::spt::profiles::{
    list_profiles, load_all_profile_stats, ProfileStatus, SptProfile, SptProfileStats,
};
use crate::web::auth::{require_auth, SessionUser};
use crate::web::error::WebError;
use crate::web::nav::NavContext;
use crate::web::state::AppState;

fn build_user_profiles(
    users: &[User],
    spt_dir: &std::path::Path,
    profile_stats: &std::collections::HashMap<String, SptProfileStats>,
) -> Vec<ProfileStatus> {
    users
        .iter()
        .map(|u| {
            let Some(ref profile_id) = u.spt_profile_id else {
                return ProfileStatus::NotFound;
            };
            if profile_id.is_empty() {
                return ProfileStatus::NotFound;
            }
            match profile_stats.get(profile_id) {
                Some(stats) => ProfileStatus::Found(stats.clone()),
                None => {
                    let profile_path = spt_dir
                        .join("SPT/user/profiles")
                        .join(format!("{}.json", profile_id));
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

fn compute_available_profiles(spt_dir: &std::path::Path, users: &[User]) -> Vec<SptProfile> {
    let all_profiles = list_profiles(spt_dir).unwrap_or_default();
    let linked_aids: std::collections::HashSet<String> = users
        .iter()
        .filter_map(|u| u.spt_profile_id.clone())
        .filter(|s| !s.is_empty())
        .collect();
    all_profiles
        .into_iter()
        .filter(|p| !linked_aids.contains(&p.aid))
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
    nav: NavContext,
    roles: Vec<RoleRecord>,
    available_profiles: Vec<SptProfile>,
}

#[derive(Template)]
#[template(path = "admin/partials/users.html")]
struct UsersPartialTemplate {
    users: Vec<(User, ProfileStatus)>,
    current_user_id: i64,
    csrf_token: String,
    roles: Vec<RoleRecord>,
    available_profiles: Vec<SptProfile>,
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
    roles: Vec<RoleRecord>,
    available_profiles: Vec<SptProfile>,
}

// InviteView -- pre-computed view struct for invites template
// (Askama can't call free functions, so we pre-compute status)
pub struct InviteView {
    pub id: i64,
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
        } else if crate::web::invite::is_invite_expired(ic.invite.expires_at.as_deref()) {
            "expired"
        } else {
            "available"
        };
        InviteView {
            id: ic.invite.id,
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
    external_url: Option<String>,
}

#[derive(Template)]
#[template(path = "admin/partials/roles.html")]
struct RolesPartialTemplate {
    roles: Vec<RoleWithPermissions>,
    all_permissions: Vec<PermissionInfo>,
    csrf_token: String,
    message: Option<String>,
    error: Option<String>,
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

#[derive(serde::Deserialize)]
pub struct CreateRoleForm {
    name: String,
    display_name: String,
    #[serde(default)]
    permissions: Vec<String>,
    csrf_token: String,
}

#[derive(serde::Deserialize)]
pub struct UpdatePermissionsForm {
    #[serde(default)]
    permissions: Vec<String>,
    csrf_token: String,
}

// -- Helpers --

fn build_permission_info_list(
    checked: &std::collections::HashSet<Permission>,
) -> Vec<PermissionInfo> {
    Permission::ALL
        .iter()
        .map(|p| PermissionInfo {
            slug: p.as_str(),
            display_name: p.display_name(),
            category: p.category(),
            checked: checked.contains(p),
        })
        .collect()
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

    if !user.has_permission(Permission::UsersManage) {
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
    crate::web::auth::require_permission(&user, Permission::UsersManage)?;
    let csrf_token = crate::web::csrf::get_or_create_token(&session);
    let flash = crate::web::flash::take_flash(&session);

    let db = state.db.clone();
    let (all_users, roles) = web::block(move || {
        let db = db.lock();
        let users = db.list_users()?;
        let roles = db.list_roles()?;
        Ok::<_, rusqlite::Error>((users, roles))
    })
    .await
    .map_err(WebError::from)?
    .map_err(WebError::from)?;

    let spt_dir = state.spt_dir.clone();
    let profile_stats = web::block(move || load_all_profile_stats(&spt_dir))
        .await
        .map_err(WebError::from)?;

    let profiles = build_user_profiles(&all_users, &state.spt_dir, &profile_stats);
    let available_profiles = compute_available_profiles(&state.spt_dir, &all_users);
    let users: Vec<(User, ProfileStatus)> = all_users.into_iter().zip(profiles).collect();
    let current_user_id = user.user_id;

    let tmpl = AdminPageTemplate {
        user,
        csrf_token,
        users,
        current_user_id,
        flash,
        nav: NavContext::from_state(&state),
        roles,
        available_profiles,
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
    let (all_users, roles) = web::block(move || {
        let db = db.lock();
        let users = db.list_users()?;
        let roles = db.list_roles()?;
        Ok::<_, rusqlite::Error>((users, roles))
    })
    .await
    .map_err(WebError::from)?
    .map_err(WebError::from)?;

    let spt_dir = state.spt_dir.clone();
    let profile_stats = web::block(move || load_all_profile_stats(&spt_dir))
        .await
        .map_err(WebError::from)?;

    let profiles = build_user_profiles(&all_users, &state.spt_dir, &profile_stats);
    let available_profiles = compute_available_profiles(&state.spt_dir, &all_users);
    let users: Vec<(User, ProfileStatus)> = all_users.into_iter().zip(profiles).collect();

    let tmpl = UsersPartialTemplate {
        users,
        current_user_id: user.user_id,
        csrf_token,
        roles,
        available_profiles,
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

    // Validate role exists in DB
    let role_name = form.role.clone();
    let db = state.db.clone();
    let role_exists = web::block(move || {
        let db = db.lock();
        db.get_role_by_name(&role_name)
    })
    .await
    .map_err(WebError::from)?
    .map_err(WebError::from)?;

    if role_exists.is_none() {
        return Err(WebError::BadRequest("Unknown role".to_string()).into());
    }

    let new_role = form.role.clone();
    let db = state.db.clone();
    let affected = web::block(move || {
        let db = db.lock();
        db.update_user_role(target_id, &new_role)
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
    let reset_link = format!("{scheme}://{host}/quma/reset-password?token={token}");
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

pub async fn delete_user(
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
        return Err(WebError::BadRequest("Cannot delete yourself".to_string()).into());
    }

    let db = state.db.clone();
    let result = web::block(move || {
        let db = db.lock();
        db.delete_user(target_id)
    })
    .await
    .map_err(WebError::from)?
    .map_err(WebError::from)?;

    match result {
        DeleteUserResult::Deleted => admin_users(state, req, session).await,
        DeleteUserResult::LastAdmin => {
            Err(WebError::UnprocessableEntity("Cannot delete the last admin".to_string()).into())
        }
        DeleteUserResult::NotFound => Err(WebError::NotFound.into()),
    }
}

#[derive(serde::Deserialize)]
pub struct LinkProfileForm {
    pub csrf_token: String,
    pub spt_profile_id: Option<String>,
}

pub async fn link_profile(
    req: HttpRequest,
    session: Session,
    state: Data<AppState>,
    path: Path<i64>,
    form: Form<LinkProfileForm>,
) -> actix_web::Result<Html> {
    let current_user = require_auth(&req)?;
    crate::web::auth::require_permission(&current_user, Permission::UsersManage)?;
    if !crate::web::csrf::validate_token(&session, &form.csrf_token) {
        return Err(WebError::Forbidden.into());
    }

    let target_id = path.into_inner();

    let profile_id = form.spt_profile_id.clone().filter(|s| !s.is_empty());

    if let Some(ref aid) = profile_id {
        if !aid
            .chars()
            .all(|c| c.is_alphanumeric() || c == '-' || c == '_')
        {
            return Err(WebError::BadRequest("Invalid profile AID format".to_string()).into());
        }

        let spt_dir = state.spt_dir.clone();
        let aid_check = aid.clone();
        let db_check = state.db.clone();
        let (exists, already_linked) = web::block(move || {
            let profile_path = spt_dir
                .join("SPT/user/profiles")
                .join(format!("{aid_check}.json"));
            let exists = profile_path.exists();
            let db = db_check.lock();
            let already_linked = db
                .get_user_by_spt_profile_id(&aid_check)
                .ok()
                .flatten()
                .map(|u| u.id != target_id)
                .unwrap_or(false);
            (exists, already_linked)
        })
        .await
        .map_err(WebError::from)?;

        if !exists {
            return Err(
                WebError::BadRequest("Profile AID does not exist on disk".to_string()).into(),
            );
        }
        if already_linked {
            return Err(WebError::BadRequest(
                "Profile is already linked to another user".to_string(),
            )
            .into());
        }
    }

    let db = state.db.clone();
    let pid = profile_id.clone();
    web::block(move || {
        let db = db.lock();
        db.update_user_spt_profile_id(target_id, pid.as_deref())
    })
    .await
    .map_err(WebError::from)?
    .map_err(WebError::from)?;

    let msg = if profile_id.is_some() {
        "Profile linked"
    } else {
        "Profile unlinked"
    };

    render_user_row(
        &state,
        &session,
        target_id,
        current_user.user_id,
        None,
        Some(msg.to_string()),
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
    let external_url = state.config.read().external_url.clone();

    let tmpl = InvitesPartialTemplate {
        invites,
        csrf_token,
        highlight_code: None,
        external_url,
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
    let external_url = state.config.read().external_url.clone();

    let tmpl = InvitesPartialTemplate {
        invites,
        csrf_token,
        highlight_code: Some(code),
        external_url,
    };
    Ok(Html::new(tmpl.render().map_err(WebError::from)?))
}

pub async fn delete_invite(
    state: Data<AppState>,
    req: HttpRequest,
    session: Session,
    path: Path<i64>,
    form: Form<CsrfOnly>,
) -> actix_web::Result<Html> {
    require_auth(&req)?;
    if !crate::web::csrf::validate_token(&session, &form.csrf_token) {
        return Err(WebError::Forbidden.into());
    }

    let invite_id = path.into_inner();
    let db = state.db.clone();
    let result = web::block(move || {
        let db = db.lock();
        db.delete_invite(invite_id)
    })
    .await
    .map_err(WebError::from)?
    .map_err(WebError::from)?;

    match result {
        DeleteInviteResult::Deleted => admin_invites(state, req, session).await,
        DeleteInviteResult::AlreadyUsed => Err(WebError::UnprocessableEntity(
            "Cannot delete a used invite code".to_string(),
        )
        .into()),
        DeleteInviteResult::NotFound => Err(WebError::NotFound.into()),
    }
}

// -- Role management handlers --

async fn render_roles_partial(
    state: &Data<AppState>,
    session: &Session,
    message: Option<String>,
    error: Option<String>,
) -> actix_web::Result<Html> {
    let csrf_token = crate::web::csrf::get_or_create_token(session);
    let db = state.db.clone();
    let roles = web::block(move || {
        let db = db.lock();
        db.list_roles_with_permissions()
    })
    .await
    .map_err(WebError::from)?
    .map_err(WebError::from)?;

    let all_permissions = build_permission_info_list(&std::collections::HashSet::new());

    let tmpl = RolesPartialTemplate {
        roles,
        all_permissions,
        csrf_token,
        message,
        error,
    };
    Ok(Html::new(tmpl.render().map_err(WebError::from)?))
}

pub async fn admin_roles(
    state: Data<AppState>,
    req: HttpRequest,
    session: Session,
) -> actix_web::Result<Html> {
    let user = require_auth(&req)?;
    crate::web::auth::require_permission(&user, Permission::UsersManage)?;
    render_roles_partial(&state, &session, None, None).await
}

pub async fn create_role_handler(
    state: Data<AppState>,
    req: HttpRequest,
    session: Session,
    form: Form<CreateRoleForm>,
) -> actix_web::Result<Html> {
    let user = require_auth(&req)?;
    crate::web::auth::require_permission(&user, Permission::UsersManage)?;
    if !crate::web::csrf::validate_token(&session, &form.csrf_token) {
        return Err(WebError::Forbidden.into());
    }

    // Validate role name
    if let Err(msg) = validate_role_name(&form.name) {
        return render_roles_partial(&state, &session, None, Some(msg.to_string())).await;
    }

    // Validate display_name: 1-64 chars, no control chars
    let display_name = form.display_name.trim();
    if display_name.is_empty() || display_name.len() > 64 {
        return render_roles_partial(
            &state,
            &session,
            None,
            Some("Display name must be 1-64 characters".to_string()),
        )
        .await;
    }
    if display_name.chars().any(|c| c.is_control()) {
        return render_roles_partial(
            &state,
            &session,
            None,
            Some("Display name cannot contain control characters".to_string()),
        )
        .await;
    }

    // Parse permission slugs
    let permissions: Vec<Permission> = form
        .permissions
        .iter()
        .filter_map(|s| Permission::from_slug(s))
        .collect();

    let name = form.name.clone();
    let dn = display_name.to_string();
    let db = state.db.clone();
    let result = web::block(move || {
        let db = db.lock();
        db.create_role(&name, &dn, &permissions)
    })
    .await
    .map_err(WebError::from)?;

    match result {
        Ok(_) => {
            render_roles_partial(
                &state,
                &session,
                Some(format!("Role \"{}\" created", form.name)),
                None,
            )
            .await
        }
        Err(e) => {
            let msg = if e.to_string().contains("UNIQUE") {
                format!("Role \"{}\" already exists", form.name)
            } else {
                format!("Failed to create role: {e}")
            };
            render_roles_partial(&state, &session, None, Some(msg)).await
        }
    }
}

pub async fn update_role_handler(
    state: Data<AppState>,
    req: HttpRequest,
    session: Session,
    path: Path<String>,
    form: Form<UpdatePermissionsForm>,
) -> actix_web::Result<Html> {
    let user = require_auth(&req)?;
    crate::web::auth::require_permission(&user, Permission::UsersManage)?;
    if !crate::web::csrf::validate_token(&session, &form.csrf_token) {
        return Err(WebError::Forbidden.into());
    }

    let role_name = path.into_inner();

    // Parse permissions from form
    let permissions: Vec<Permission> = form
        .permissions
        .iter()
        .filter_map(|s| Permission::from_slug(s))
        .collect();

    let rn = role_name.clone();
    let db = state.db.clone();
    let result = web::block(move || {
        let db = db.lock();
        db.update_role_permissions(&rn, &permissions)
    })
    .await
    .map_err(WebError::from)?
    .map_err(WebError::from)?;

    use crate::db::rbac::UpdatePermissionsResult;
    match result {
        UpdatePermissionsResult::AdminImmutable => {
            return render_roles_partial(
                &state,
                &session,
                None,
                Some("Admin role permissions cannot be modified".to_string()),
            )
            .await;
        }
        UpdatePermissionsResult::WouldRemoveLastAdmin => {
            return render_roles_partial(
                &state,
                &session,
                None,
                Some("Cannot remove users.manage — would leave no admin-capable users".to_string()),
            )
            .await;
        }
        UpdatePermissionsResult::Updated => {}
    }

    render_roles_partial(
        &state,
        &session,
        Some(format!("Permissions updated for \"{}\"", role_name)),
        None,
    )
    .await
}

pub async fn delete_role_handler(
    state: Data<AppState>,
    req: HttpRequest,
    session: Session,
    path: Path<String>,
    form: Form<CsrfOnly>,
) -> actix_web::Result<Html> {
    let user = require_auth(&req)?;
    crate::web::auth::require_permission(&user, Permission::UsersManage)?;
    if !crate::web::csrf::validate_token(&session, &form.csrf_token) {
        return Err(WebError::Forbidden.into());
    }

    let role_name = path.into_inner();

    let rn = role_name.clone();
    let db = state.db.clone();
    let result = web::block(move || {
        let db = db.lock();
        db.delete_role(&rn)
    })
    .await
    .map_err(WebError::from)?
    .map_err(WebError::from)?;

    match result {
        DeleteRoleResult::Deleted => {
            render_roles_partial(
                &state,
                &session,
                Some(format!("Role \"{}\" deleted", role_name)),
                None,
            )
            .await
        }
        DeleteRoleResult::BuiltIn => {
            render_roles_partial(
                &state,
                &session,
                None,
                Some("Cannot delete built-in role".to_string()),
            )
            .await
        }
        DeleteRoleResult::HasUsers(n) => {
            render_roles_partial(
                &state,
                &session,
                None,
                Some(format!(
                    "Cannot delete: {n} user{} assigned to this role",
                    if n == 1 { " is" } else { "s are" }
                )),
            )
            .await
        }
        DeleteRoleResult::NotFound => {
            render_roles_partial(&state, &session, None, Some("Role not found".to_string())).await
        }
    }
}

// -- User row helpers --

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
    let (user, roles) = web::block(move || {
        let db = db.lock();
        let user = db.get_user_by_id(user_id)?;
        let roles = db.list_roles()?;
        Ok::<_, rusqlite::Error>((user, roles))
    })
    .await
    .map_err(WebError::from)?
    .map_err(WebError::from)?;
    let user = user.ok_or(WebError::NotFound)?;

    let spt_dir = state.spt_dir.clone();
    let aid = user.spt_profile_id.clone().unwrap_or_default();
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

    let spt_dir2 = state.spt_dir.clone();
    let db2 = state.db.clone();
    let available_profiles = web::block(move || {
        let db = db2.lock();
        let users = db.list_users().unwrap_or_default();
        compute_available_profiles(&spt_dir2, &users)
    })
    .await
    .map_err(WebError::from)?;

    let tmpl = UserRowTemplate {
        u: user,
        profile,
        current_user_id,
        csrf_token,
        reset_link,
        row_message,
        roles,
        available_profiles,
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
