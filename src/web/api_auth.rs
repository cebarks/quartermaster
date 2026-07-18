use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::Path;

use actix_web::body::BoxBody;
use actix_web::dev::{ServiceRequest, ServiceResponse};
use actix_web::middleware::Next;
use actix_web::{web, HttpMessage};
use rand::distr::Alphanumeric;
use rand::RngExt;
use subtle::ConstantTimeEq;

use crate::dirs::QumaDirs;
use crate::web::auth::SessionUser;

pub struct ApiTokenState {
    pub token: String,
}

/// Generate a random API token and write it to `<spt_dir>/.quma/api-token` with mode 0600.
/// Returns the token string.
pub fn generate_api_token(
    dirs: &QumaDirs,
    bind: &str,
    port: u16,
    tls: bool,
) -> anyhow::Result<String> {
    let token: String = rand::rng()
        .sample_iter(Alphanumeric)
        .take(64)
        .map(char::from)
        .collect();

    let connect_host = if bind == "0.0.0.0" || bind == "::" {
        "127.0.0.1"
    } else {
        bind
    };
    let scheme = if tls { "https" } else { "http" };
    let url = format!("{scheme}://{connect_host}:{port}");

    let token_dir = dirs.spt_server.join(".quma");
    fs::create_dir_all(&token_dir)?;
    let token_path = token_dir.join("api-token");
    let content = format!("token = {token}\nurl = {url}\n");
    fs::write(&token_path, &content)?;
    fs::set_permissions(&token_path, fs::Permissions::from_mode(0o600))?;

    tracing::info!(path = %token_path.display(), "API token written");
    Ok(token)
}

/// Read API token file and return (token, url).
pub fn read_api_token(spt_dir: &Path) -> anyhow::Result<(String, String)> {
    let token_path = spt_dir.join(".quma/api-token");
    let content = fs::read_to_string(&token_path)?;
    let mut token = None;
    let mut url = None;
    for line in content.lines() {
        if let Some(val) = line.strip_prefix("token = ") {
            token = Some(val.trim().to_string());
        } else if let Some(val) = line.strip_prefix("url = ") {
            url = Some(val.trim().to_string());
        }
    }
    Ok((
        token.ok_or_else(|| anyhow::anyhow!("missing token in api-token file"))?,
        url.ok_or_else(|| anyhow::anyhow!("missing url in api-token file"))?,
    ))
}

/// Middleware that validates `X-Quma-Token` header. If present and valid, inject a synthetic
/// admin SessionUser. If absent, fall through to session auth.
pub async fn api_auth_middleware(
    req: ServiceRequest,
    next: Next<BoxBody>,
) -> Result<ServiceResponse<BoxBody>, actix_web::Error> {
    if let Some(header) = req.headers().get("X-Quma-Token") {
        if let Some(token_state) = req.app_data::<web::Data<ApiTokenState>>() {
            if header.as_bytes().ct_eq(token_state.token.as_bytes()).into() {
                // Valid token — inject synthetic admin user
                let synthetic_user = SessionUser {
                    user_id: -1, // sentinel for API token auth
                    username: "api-token".to_string(),
                    role_name: "admin".to_string(),
                    role_display_name: "Admin".to_string(),
                    permissions: crate::db::rbac::Permission::ALL.iter().copied().collect(),
                    has_password: false,
                };
                req.extensions_mut().insert(synthetic_user);
                return next.call(req).await;
            }
        }
        // Invalid token
        return Err(actix_web::error::ErrorUnauthorized("Invalid API token"));
    }
    // No token header — fall through to session auth
    next.call(req).await
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[test]
    fn test_generate_api_token() {
        let tmp = tempdir().unwrap();
        let dirs = QumaDirs::from_root(tmp.path().to_path_buf());

        let token = generate_api_token(&dirs, "0.0.0.0", 9190, true).unwrap();
        assert_eq!(token.len(), 64);

        let token_path = dirs.spt_server.join(".quma/api-token");
        assert!(token_path.exists());

        let content = fs::read_to_string(&token_path).unwrap();
        assert!(content.contains(&format!("token = {token}")));
        assert!(content.contains("url = https://127.0.0.1:9190"));

        let metadata = fs::metadata(&token_path).unwrap();
        let mode = metadata.permissions().mode();
        assert_eq!(mode & 0o777, 0o600, "expected mode 0600, got {mode:o}");
    }

    #[test]
    fn test_read_api_token() {
        let tmp = tempdir().unwrap();
        let dirs = QumaDirs::from_root(tmp.path().to_path_buf());

        let expected_token = generate_api_token(&dirs, "192.168.1.1", 8080, false).unwrap();
        let (token, url) = read_api_token(&dirs.spt_server).unwrap();

        assert_eq!(token, expected_token);
        assert_eq!(url, "http://192.168.1.1:8080");
    }
}
