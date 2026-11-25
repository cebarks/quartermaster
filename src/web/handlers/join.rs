use actix_session::Session;
use actix_web::web::Form;
use actix_web::{web, HttpRequest, HttpResponse};
use askama::Template;

use crate::web::auth::{hash_password, validate_password_complexity};
use crate::web::bootstrap::{for_instance, generate_bash_script, generate_powershell_script};
use crate::web::error::WebError;
use crate::web::invite::validate_invite_code;
use crate::web::state::AppState;

pub(crate) const DEFAULT_SERVER_NAME: &str = "SPT Server";

const SPT_EDITIONS: &[&str] = &[
    "Standard",
    "Left Behind",
    "Prepare for Escape",
    "Edge of Darkness",
    "The Unheard Edition",
];

#[derive(serde::Deserialize)]
pub struct JoinForm {
    code: String,
    username: String,
    password: String,
    password_confirm: String,
    edition: String,
    csrf_token: String,
}

#[derive(Debug, serde::Deserialize)]
pub struct JoinQuery {
    pub code: Option<String>,
}

#[derive(Template)]
#[template(path = "join.html")]
struct JoinTemplate {
    server_name: String,
    spt_version: String,
    fika_installed: bool,
    mod_count: usize,
    code: String,
    error: Option<String>,
    csrf_token: String,
    editions: &'static [&'static str],
    convoy_installed: bool,
}

fn referrer_policy(resp: HttpResponse) -> HttpResponse {
    let mut resp = resp;
    resp.headers_mut().insert(
        actix_web::http::header::HeaderName::from_static("referrer-policy"),
        actix_web::http::header::HeaderValue::from_static("no-referrer"),
    );
    resp
}

struct JoinErrorContext {
    code: String,
    server_name: String,
    spt_version: String,
    fika_installed: bool,
    mod_count: usize,
    csrf_token: String,
    convoy_installed: bool,
}

pub async fn join_page(
    query: web::Query<JoinQuery>,
    state: web::Data<AppState>,
    session: Session,
) -> actix_web::Result<HttpResponse> {
    let csrf_token = crate::web::csrf::get_or_create_token(&session);
    let code = query.code.clone().unwrap_or_default();

    // Validate invite code
    let db = state.db.clone();
    let code_clone = code.clone();
    let invite_result = web::block(move || {
        let db = db.lock();
        validate_invite_code(&db, &code_clone)
    })
    .await
    .map_err(WebError::from)?;

    if let Err(e) = invite_result {
        let tmpl = JoinTemplate {
            server_name: DEFAULT_SERVER_NAME.to_string(),
            spt_version: state.spt_info.spt_version.clone(),
            fika_installed: state.fika_installed,
            mod_count: 0,
            code,
            error: Some(e.to_string()),
            csrf_token,
            editions: SPT_EDITIONS,
            convoy_installed: false,
        };
        return Ok(referrer_policy(
            HttpResponse::BadRequest()
                .content_type("text/html")
                .body(tmpl.render().map_err(WebError::from)?),
        ));
    }

    let server_name = {
        let config = state.config.read();
        config
            .server_name
            .clone()
            .unwrap_or_else(|| DEFAULT_SERVER_NAME.to_string())
    };

    // Count client-syncable mods
    let db = state.db.clone();
    let mod_count = web::block(move || {
        let db = db.lock();
        db.count_client_syncable_mods()
    })
    .await
    .map_err(WebError::from)?
    .map_err(WebError::from)?;

    let db = state.db.clone();
    let convoy_installed = web::block(move || {
        let db = db.lock();
        db.is_forge_mod_installed(2806)
    })
    .await
    .map_err(WebError::from)?
    .map_err(WebError::from)?;

    let tmpl = JoinTemplate {
        server_name,
        spt_version: state.spt_info.spt_version.clone(),
        fika_installed: state.fika_installed,
        mod_count,
        code,
        error: None,
        csrf_token,
        editions: SPT_EDITIONS,
        convoy_installed,
    };

    Ok(referrer_policy(
        HttpResponse::Ok()
            .content_type("text/html")
            .body(tmpl.render().map_err(WebError::from)?),
    ))
}

fn render_join_error(msg: &str, ctx: JoinErrorContext) -> actix_web::Result<HttpResponse> {
    let tmpl = JoinTemplate {
        server_name: ctx.server_name,
        spt_version: ctx.spt_version,
        fika_installed: ctx.fika_installed,
        mod_count: ctx.mod_count,
        code: ctx.code,
        error: Some(msg.to_string()),
        csrf_token: ctx.csrf_token,
        editions: SPT_EDITIONS,
        convoy_installed: ctx.convoy_installed,
    };
    Ok(referrer_policy(
        HttpResponse::BadRequest()
            .content_type("text/html")
            .body(tmpl.render().map_err(WebError::from)?),
    ))
}

pub async fn join_submit(
    form: Form<JoinForm>,
    state: web::Data<AppState>,
    session: Session,
) -> actix_web::Result<HttpResponse> {
    let form = form.into_inner();

    if !crate::web::csrf::validate_token(&session, &form.csrf_token) {
        return Err(WebError::Forbidden.into());
    }

    let csrf_token = crate::web::csrf::get_or_create_token(&session);

    let server_name = {
        let config = state.config.read();
        config
            .server_name
            .clone()
            .unwrap_or_else(|| DEFAULT_SERVER_NAME.to_string())
    };
    let spt_version = state.spt_info.spt_version.clone();
    let fika_installed = state.fika_installed;

    // Count mods for error re-renders
    let db = state.db.clone();
    let mod_count = web::block(move || {
        let db = db.lock();
        db.count_client_syncable_mods()
    })
    .await
    .map_err(WebError::from)?
    .map_err(WebError::from)?;

    let db = state.db.clone();
    let convoy_installed = web::block(move || {
        let db = db.lock();
        db.is_forge_mod_installed(2806)
    })
    .await
    .map_err(WebError::from)?
    .map_err(WebError::from)?;

    let code = form.code.clone();
    let render_err = |msg: &str| {
        render_join_error(
            msg,
            JoinErrorContext {
                code: code.clone(),
                server_name: server_name.clone(),
                spt_version: spt_version.clone(),
                fika_installed,
                mod_count,
                csrf_token: csrf_token.clone(),
                convoy_installed,
            },
        )
    };

    // Validate invite code
    let db = state.db.clone();
    let code_clone = form.code.clone();
    let invite_result = web::block(move || {
        let db = db.lock();
        validate_invite_code(&db, &code_clone)
    })
    .await
    .map_err(WebError::from)?;

    if let Err(e) = invite_result {
        return render_err(&e.to_string());
    }

    // Validate username
    let username = form.username.trim().to_string();
    if username.is_empty() || username.len() > 32 {
        return render_err("Username must be 1-32 characters");
    }
    if !username.chars().all(|c| c.is_alphanumeric() || c == '_') {
        return render_err("Username can only contain letters, numbers, and underscores");
    }
    if username.starts_with("headless_") {
        return render_err("Username cannot start with 'headless_'");
    }

    // Validate password
    if let Err(msg) = validate_password_complexity(&form.password) {
        return render_err(msg);
    }
    if form.password != form.password_confirm {
        return render_err("Passwords do not match");
    }

    // Validate edition
    if !SPT_EDITIONS.contains(&form.edition.as_str()) {
        return render_err("Invalid edition selection");
    }

    // Check if username already exists (any state — no claim flow)
    let db = state.db.clone();
    let username_clone = username.clone();
    let existing_user = web::block(move || {
        let db = db.lock();
        db.get_user_by_username(&username_clone)
    })
    .await
    .map_err(WebError::from)?
    .map_err(WebError::from)?;

    if existing_user.is_some() {
        return render_err("Username is already taken");
    }

    // Create SPT profile via server API
    let (host, port) = crate::server_detect::resolve_server_addr(&state.config(), &state.dirs);
    let spt_client = match crate::spt::server::SptClient::new(&host, port) {
        Ok(c) => c,
        Err(_) => {
            return render_err("Could not connect to the SPT server. Make sure it is running.");
        }
    };

    // TODO(debt): If this succeeds but the DB transaction below fails, the SPT
    // profile is orphaned (no Quartermaster account links to it).
    let profile_aid = match spt_client.register_profile(&username, &form.edition).await {
        Ok(aid) => {
            let trimmed = aid.trim().trim_matches('"').to_string();
            if trimmed.is_empty() {
                tracing::error!(username = %username, "SPT server returned empty AID — profile creation failed");
                return render_err(
                    "Could not create your SPT profile — the server returned an invalid response. Please try again or contact an administrator.",
                );
            }
            trimmed
        }
        Err(e) => {
            tracing::warn!(err = %e, username = %username, "SPT profile registration failed");
            return render_err(
                "Could not create your SPT profile. Make sure the SPT server is running.",
            );
        }
    };

    // Hash password
    let password = form.password.clone();
    let password_hash = web::block(move || hash_password(&password))
        .await
        .map_err(WebError::from)?
        .map_err(WebError::from)?;

    // Create user + consume invite in a single transaction
    let username_for_login = username.clone();
    let db = state.db.clone();
    let code_for_invite = code.clone();
    let result = web::block(move || {
        let db = db.lock();
        let tx = db.begin_transaction()?;

        // Double-check username not taken (race condition guard)
        if db.get_user_by_username(&username)?.is_some() {
            // tx rolls back on drop
            return Ok::<_, rusqlite::Error>(Err("Username is already taken".to_string()));
        }

        // Create user first so we have a real user_id for the FK on invite_codes.used_by
        let user_id = db.insert_user(
            &username,
            Some(&profile_aid),
            Some(&password_hash),
            "player",
            false,
        )?;

        let used = db.use_invite(&code_for_invite, user_id)?;
        if used == 0 {
            // tx rolls back on drop — removes the user too
            return Ok(Err("Invite code is invalid or expired".to_string()));
        }

        tx.commit()?;
        Ok(Ok(()))
    })
    .await
    .map_err(WebError::from)?
    .map_err(WebError::from)?;

    match result {
        Ok(()) => {
            let db = state.db.clone();
            let uname = username_for_login;
            let new_user = web::block(move || {
                let db = db.lock();
                db.get_user_by_username(&uname)
            })
            .await
            .map_err(WebError::from)?
            .map_err(WebError::from)?;

            if let Some(new_user) = new_user {
                let db = state.db.clone();
                let role = new_user.role.clone();
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
                let session_user = crate::web::auth::SessionUser {
                    user_id: new_user.id,
                    has_password: new_user.password_hash.is_some(),
                    username: new_user.username,
                    role_name: new_user.role,
                    role_display_name: role_display,
                    permissions,
                };
                if let Err(e) = crate::web::auth::set_session_user(&session, &session_user) {
                    tracing::warn!(err = %e, "failed to auto-login after registration, falling back to login page");
                    crate::web::flash::set_flash(
                        &session,
                        "Account created — please log in.",
                        crate::web::flash::FlashType::Success,
                    );
                    return Ok(referrer_policy(
                        HttpResponse::SeeOther()
                            .insert_header(("Location", "/quma/login"))
                            .finish(),
                    ));
                }

                Ok(referrer_policy(
                    HttpResponse::SeeOther()
                        .insert_header(("Location", "/quma/setup"))
                        .finish(),
                ))
            } else {
                crate::web::flash::set_flash(
                    &session,
                    "Account created — please log in.",
                    crate::web::flash::FlashType::Success,
                );
                Ok(referrer_policy(
                    HttpResponse::SeeOther()
                        .insert_header(("Location", "/quma/login"))
                        .finish(),
                ))
            }
        }
        Err(msg) => render_err(&msg),
    }
}

/// Serve the full filtered client pack after validating the invite.
pub async fn mod_archive(
    query: web::Query<JoinQuery>,
    state: web::Data<AppState>,
    req: HttpRequest,
) -> actix_web::Result<HttpResponse> {
    let code = query.code.clone().unwrap_or_default();

    // Validate invite
    let db = state.db.clone();
    let code_clone = code.clone();
    let invite_result = web::block(move || {
        let db = db.lock();
        validate_invite_code(&db, &code_clone)
    })
    .await
    .map_err(WebError::from)?;

    if let Err(e) = invite_result {
        return Ok(referrer_policy(
            HttpResponse::BadRequest()
                .content_type("text/plain")
                .body(e.to_string()),
        ));
    }

    // The complete filtered instance pack contains Fika and dependencies too.
    // Reuse setup's disk cache/Range behavior, while retaining invite validation.
    let response = crate::web::handlers::setup::setup_mods_zip(state, req).await?;
    Ok(referrer_policy(response))
}

pub async fn bootstrap_bash(
    query: web::Query<JoinQuery>,
    state: web::Data<AppState>,
) -> actix_web::Result<HttpResponse> {
    let code = query.code.clone().unwrap_or_default();

    let db = state.db.clone();
    let code_clone = code.clone();
    let invite_result = web::block(move || {
        let db = db.lock();
        validate_invite_code(&db, &code_clone)
    })
    .await
    .map_err(WebError::from)?;

    if let Err(e) = invite_result {
        return Ok(referrer_policy(
            HttpResponse::BadRequest()
                .content_type("text/plain")
                .body(e.to_string()),
        ));
    }

    let (external_url, server_name) = {
        let config = state.config.read();
        let external_url = match &config.external_url {
            Some(url) if !url.is_empty() => url.clone(),
            _ => {
                return Ok(referrer_policy(
                    HttpResponse::ServiceUnavailable()
                        .content_type("text/plain")
                        .body("Bootstrap not configured: external_url is required"),
                ));
            }
        };
        let server_name = config
            .server_name
            .clone()
            .unwrap_or_else(|| DEFAULT_SERVER_NAME.to_string());
        (external_url, server_name)
    };

    let spt_server_url = build_spt_server_url(&external_url);

    let archive_url = format!(
        "{}/quma/join/mods.zip?code={}",
        external_url.trim_end_matches('/'),
        code
    );
    let bootstrap = for_instance(&state, server_name, archive_url, spt_server_url).await?;
    let script = generate_bash_script(&bootstrap);

    Ok(referrer_policy(
        HttpResponse::Ok()
            .content_type("text/x-shellscript")
            .insert_header((
                "content-disposition",
                "attachment; filename=\"quma-bootstrap.sh\"",
            ))
            .body(script),
    ))
}

pub async fn bootstrap_powershell(
    query: web::Query<JoinQuery>,
    state: web::Data<AppState>,
) -> actix_web::Result<HttpResponse> {
    let code = query.code.clone().unwrap_or_default();

    let db = state.db.clone();
    let code_clone = code.clone();
    let invite_result = web::block(move || {
        let db = db.lock();
        validate_invite_code(&db, &code_clone)
    })
    .await
    .map_err(WebError::from)?;

    if let Err(e) = invite_result {
        return Ok(referrer_policy(
            HttpResponse::BadRequest()
                .content_type("text/plain")
                .body(e.to_string()),
        ));
    }

    let (external_url, server_name) = {
        let config = state.config.read();
        let external_url = match &config.external_url {
            Some(url) if !url.is_empty() => url.clone(),
            _ => {
                return Ok(referrer_policy(
                    HttpResponse::ServiceUnavailable()
                        .content_type("text/plain")
                        .body("Bootstrap not configured: external_url is required"),
                ));
            }
        };
        let server_name = config
            .server_name
            .clone()
            .unwrap_or_else(|| DEFAULT_SERVER_NAME.to_string());
        (external_url, server_name)
    };

    let spt_server_url = build_spt_server_url(&external_url);

    let archive_url = format!(
        "{}/quma/join/mods.zip?code={}",
        external_url.trim_end_matches('/'),
        code
    );
    let bootstrap = for_instance(&state, server_name, archive_url, spt_server_url).await?;
    let script = generate_powershell_script(&bootstrap);

    Ok(referrer_policy(
        HttpResponse::Ok()
            .content_type("text/plain")
            .insert_header((
                "content-disposition",
                "attachment; filename=\"quma-bootstrap.ps1\"",
            ))
            .body(script),
    ))
}

pub(crate) fn build_spt_server_url(external_url: &str) -> String {
    let trimmed = external_url.trim_end_matches('/');
    let without_scheme = trimmed
        .strip_prefix("https://")
        .or_else(|| trimmed.strip_prefix("http://"))
        .unwrap_or(trimmed);
    format!("https://{}", without_scheme)
}

pub(crate) fn build_mod_zip(
    dirs: &crate::dirs::QumaDirs,
    files: &[crate::db::mods::InstalledFile],
) -> anyhow::Result<Vec<u8>> {
    let cursor = crate::web::mod_zip_cache::build_mod_zip_to_writer(
        dirs,
        files,
        std::io::Cursor::new(Vec::new()),
        false,
    )?;
    Ok(cursor.into_inner())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn build_spt_server_url_normalizes_external_url() {
        assert_eq!(
            build_spt_server_url("https://tarkov.example.com"),
            "https://tarkov.example.com"
        );
        assert_eq!(
            build_spt_server_url("https://tarkov.example.com/"),
            "https://tarkov.example.com"
        );
        assert_eq!(
            build_spt_server_url("http://tarkov.example.com"),
            "https://tarkov.example.com"
        );
        assert_eq!(
            build_spt_server_url("https://tarkov.example.com:443"),
            "https://tarkov.example.com:443"
        );
    }

    #[test]
    fn build_spt_server_url_rejects_empty() {
        let result = build_spt_server_url("");
        assert_eq!(
            result, "https://",
            "empty input produces invalid URL — callers must guard"
        );
    }

    #[test]
    fn build_mod_zip_rejects_path_traversal() {
        let spt_dir = tempfile::tempdir().unwrap();
        let secret = spt_dir.path().join("../secret.txt");
        std::fs::write(&secret, b"sensitive data").unwrap();

        let files = vec![crate::db::mods::InstalledFile {
            id: 0,
            mod_id: Some(1),
            addon_id: None,
            file_path: "../secret.txt".to_string(),
            file_hash: None,
            file_size: None,
            source: "archive".to_string(),
        }];

        let dirs = crate::dirs::QumaDirs::from_legacy(spt_dir.path().to_path_buf());
        let zip_bytes = build_mod_zip(&dirs, &files).unwrap();
        let reader = zip::ZipArchive::new(std::io::Cursor::new(zip_bytes)).unwrap();
        assert_eq!(reader.len(), 0, "traversal path should be skipped");

        std::fs::remove_file(&secret).ok();
    }

    #[test]
    fn build_mod_zip_rejects_absolute_path() {
        let spt_dir = tempfile::tempdir().unwrap();

        let files = vec![crate::db::mods::InstalledFile {
            id: 0,
            mod_id: Some(1),
            addon_id: None,
            file_path: "/etc/passwd".to_string(),
            file_hash: None,
            file_size: None,
            source: "archive".to_string(),
        }];

        let dirs = crate::dirs::QumaDirs::from_legacy(spt_dir.path().to_path_buf());
        let zip_bytes = build_mod_zip(&dirs, &files).unwrap();
        let reader = zip::ZipArchive::new(std::io::Cursor::new(zip_bytes)).unwrap();
        assert_eq!(reader.len(), 0, "absolute path should be skipped");
    }
}
