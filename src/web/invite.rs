use crate::db::users::InviteCode;
use crate::db::Database;

#[derive(Debug)]
pub enum InviteError {
    Missing,
    NotFound,
    AlreadyUsed,
    Expired,
    #[allow(dead_code)] // Field is accessed through Display impl, but clippy doesn't see it
    Db(rusqlite::Error),
}

impl std::fmt::Display for InviteError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            InviteError::Missing => write!(f, "Invite code required"),
            InviteError::NotFound => write!(f, "Invalid invite code"),
            InviteError::AlreadyUsed => write!(f, "This invite code has already been used"),
            InviteError::Expired => write!(f, "This invite code has expired"),
            InviteError::Db(_) => write!(f, "An internal error occurred. Please try again."),
        }
    }
}

pub fn is_invite_expired(expires_at: Option<&str>) -> bool {
    let Some(exp) = expires_at else {
        return false;
    };
    match chrono::DateTime::parse_from_rfc3339(exp) {
        Ok(dt) => dt < chrono::Utc::now(),
        Err(_) => exp < chrono::Utc::now().to_rfc3339().as_str(),
    }
}

pub fn validate_invite_code(db: &Database, code: &str) -> Result<InviteCode, InviteError> {
    if code.is_empty() {
        return Err(InviteError::Missing);
    }

    let invite = db
        .get_invite(code)
        .map_err(|e| {
            tracing::error!(error = %e, "database error looking up invite code");
            InviteError::Db(e)
        })?
        .ok_or(InviteError::NotFound)?;

    if invite.used_by.is_some() {
        return Err(InviteError::AlreadyUsed);
    }

    if is_invite_expired(invite.expires_at.as_deref()) {
        return Err(InviteError::Expired);
    }

    Ok(invite)
}
