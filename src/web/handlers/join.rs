use actix_session::Session;
use actix_web::web::Form;
use actix_web::{web, HttpResponse};
use askama::Template;

use crate::web::auth::{hash_password, validate_password_complexity};
use crate::web::error::WebError;
use crate::web::invite::validate_invite_code;
use crate::web::state::AppState;

pub(crate) const DEFAULT_SERVER_NAME: &str = "SPT Server";

pub(crate) const FIKA_INSTALLER_URL: &str =
    "https://github.com/project-fika/Fika-Installer/releases/latest/download/Fika-Installer.exe";

pub(crate) const BOOTSTRAP_FORGE_IDS: &[i64] = &[2806];

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

    // Generate a random SPT password (nominal auth, not stored)
    let spt_password: String = {
        use rand::RngExt;
        rand::rng()
            .sample_iter(&rand::distr::Alphanumeric)
            .take(32)
            .map(char::from)
            .collect()
    };

    // TODO(debt): If this succeeds but the DB transaction below fails, the SPT
    // profile is orphaned (no Quartermaster account links to it).
    let profile_aid = match spt_client
        .register_profile(&username, &spt_password, &form.edition)
        .await
    {
        Ok(aid) => {
            let trimmed = aid.trim().trim_matches('"').to_string();
            if trimmed.is_empty() {
                tracing::warn!(username = %username, "SPT returned empty AID, falling back to scan");
                None
            } else {
                Some(trimmed)
            }
        }
        Err(e) => {
            tracing::warn!(err = %e, username = %username, "SPT profile registration failed");
            return render_err(
                "Could not create your SPT profile. Make sure the SPT server is running.",
            );
        }
    };

    // Fall back to filesystem scan if API didn't return a usable AID
    let profile_aid = match profile_aid {
        Some(aid) => Some(aid),
        None => {
            let spt_dir = state.dirs.spt_server.clone();
            let username_for_scan = username.clone();
            web::block(move || {
                for attempt in 0..5 {
                    if let Some(aid) = find_profile_by_username(&spt_dir, &username_for_scan) {
                        return Some(aid);
                    }
                    if attempt < 4 {
                        std::thread::sleep(std::time::Duration::from_millis(500));
                    }
                }
                tracing::warn!(
                    username = %username_for_scan,
                    "profile not found on disk after SPT registration — user will have no linked profile"
                );
                None
            })
            .await
            .map_err(WebError::from)?
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
            profile_aid.as_deref(),
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

/// Scan SPT profiles directory to find a profile by username.
/// Returns the AID (filename stem) if found.
fn find_profile_by_username(spt_dir: &std::path::Path, username: &str) -> Option<String> {
    let profiles_dir = spt_dir.join("SPT/user/profiles");
    let entries = match std::fs::read_dir(&profiles_dir) {
        Ok(e) => e,
        Err(e) => {
            tracing::warn!(err = %e, "failed to read profiles directory after registration");
            return None;
        }
    };

    for entry in entries.flatten() {
        let path = entry.path();
        let profile_id = match path.file_stem().and_then(|s| s.to_str()) {
            Some(id) if path.extension().and_then(|e| e.to_str()) == Some("json") => id.to_string(),
            _ => continue,
        };

        let profile_json: serde_json::Value = match std::fs::read_to_string(&path)
            .ok()
            .and_then(|s| serde_json::from_str(&s).ok())
        {
            Some(v) => v,
            None => continue,
        };

        let profile_username = profile_json
            .pointer("/info/username")
            .and_then(|v| v.as_str())
            .unwrap_or("");

        if profile_username == username {
            return Some(profile_id);
        }
    }

    None
}

pub async fn mod_archive(
    query: web::Query<JoinQuery>,
    state: web::Data<AppState>,
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

    // Get files for bootstrap mods
    let db = state.db.clone();
    let files = web::block(move || {
        let db = db.lock();
        db.get_files_for_forge_ids(BOOTSTRAP_FORGE_IDS)
    })
    .await
    .map_err(WebError::from)?
    .map_err(WebError::from)?;

    if files.is_empty() {
        return Ok(referrer_policy(
            HttpResponse::ServiceUnavailable()
                .content_type("text/plain")
                .body("No bootstrap mods (Convoy) are installed on this server"),
        ));
    }

    // Build ZIP archive in memory
    let spt_dir = state.dirs.spt_server.clone();
    let zip_bytes = web::block(move || build_mod_zip(&spt_dir, &files))
        .await
        .map_err(WebError::from)?
        .map_err(WebError::Internal)?;

    Ok(referrer_policy(
        HttpResponse::Ok()
            .content_type("application/zip")
            .insert_header((
                "content-disposition",
                "attachment; filename=\"quma-mods.zip\"",
            ))
            .body(zip_bytes),
    ))
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
    drop(config);

    let spt_server_url = build_spt_server_url(&external_url);

    let archive_url = format!(
        "{}/quma/join/mods.zip?code={}",
        external_url.trim_end_matches('/'),
        code
    );
    let script = generate_bash_script(&server_name, &archive_url, &spt_server_url);

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
    drop(config);

    let spt_server_url = build_spt_server_url(&external_url);

    let archive_url = format!(
        "{}/quma/join/mods.zip?code={}",
        external_url.trim_end_matches('/'),
        code
    );
    let script = generate_powershell_script(&server_name, &archive_url, &spt_server_url);

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

fn escape_bash(s: &str) -> String {
    s.replace('\'', "'\\''")
}

fn escape_powershell(s: &str) -> String {
    s.replace('\'', "''")
}

pub(crate) fn generate_bash_script(
    server_name: &str,
    archive_url: &str,
    spt_server_url: &str,
) -> String {
    let server_name = escape_bash(server_name);
    let archive_url = escape_bash(archive_url);
    let spt_server_url = escape_bash(spt_server_url);
    let fika_installer_url = FIKA_INSTALLER_URL;

    format!(
        r#"#!/usr/bin/env bash
set -euo pipefail

SERVER_NAME='{server_name}'
SPT_SERVER_URL='{spt_server_url}'
ARCHIVE_URL='{archive_url}'
LAUNCHER_CONFIG='SPT/user/launcher/config.json'
FIKA_INSTALLER_URL='{fika_installer_url}'

echo "=== Quartermaster Bootstrap ==="
echo "Setting up client for: $SERVER_NAME"
echo ""

# Check we're in an SPT directory
if [ ! -d "BepInEx" ]; then
    echo "ERROR: BepInEx/ directory not found."
    echo "Run this script from your SPT installation directory."
    exit 1
fi

# Check required tools
for cmd in curl unzip wine; do
    if ! command -v "$cmd" &>/dev/null; then
        echo "ERROR: '$cmd' is required but not installed."
        exit 1
    fi
done

cleanup() {{
    rm -f "${{TMPFILE:-}}" Fika-Installer.exe
}}
trap cleanup EXIT

# Step 1: Install Fika via official installer (Wine)
echo "Downloading Fika Installer..."
curl -fsSL -o Fika-Installer.exe "$FIKA_INSTALLER_URL"

echo "Installing Fika (via Wine)..."
wine Fika-Installer.exe install fika

# Step 2: Download additional mods (Convoy, etc.)
TMPFILE=$(mktemp /tmp/quma-mods-XXXXXX.zip)

echo "Downloading additional mods..."
curl -fsSL -o "$TMPFILE" "$ARCHIVE_URL"

echo "Extracting mods..."
unzip -o "$TMPFILE" -d .

# Configure launcher server address
if [ -f "$LAUNCHER_CONFIG" ]; then
    if command -v python3 &>/dev/null; then
        python3 -c "
import json, sys
with open(sys.argv[1]) as f: cfg = json.load(f)
cfg['Server'] = cfg.get('Server') or {{}}
cfg['Server']['Url'] = sys.argv[2]
with open(sys.argv[1], 'w') as f: json.dump(cfg, f, indent=2)
" "$LAUNCHER_CONFIG" "$SPT_SERVER_URL"
        echo "Launcher configured: server address set to $SPT_SERVER_URL"
    else
        echo "NOTE: python3 not found — set the server address manually in SPT Launcher:"
        echo "  $SPT_SERVER_URL"
    fi
else
    echo "NOTE: Launcher config not found at $LAUNCHER_CONFIG"
    echo "  Launch SPT once, then re-run this script, or set the server address manually:"
    echo "  $SPT_SERVER_URL"
fi

echo ""
echo "=== Setup Complete ==="
echo ""
echo "Next steps:"
echo "  1. Launch SPT and connect to: $SPT_SERVER_URL"
"#
    )
}

pub(crate) fn generate_powershell_script(
    server_name: &str,
    archive_url: &str,
    spt_server_url: &str,
) -> String {
    let server_name = escape_powershell(server_name);
    let archive_url = escape_powershell(archive_url);
    let spt_server_url = escape_powershell(spt_server_url);
    let fika_installer_url = FIKA_INSTALLER_URL;

    format!(
        r#"$ErrorActionPreference = 'Stop'

$ServerName = '{server_name}'
$SptServerUrl = '{spt_server_url}'
$ArchiveUrl = '{archive_url}'
$LauncherConfig = 'SPT\user\launcher\config.json'
$FikaInstallerUrl = '{fika_installer_url}'

Write-Host "=== Quartermaster Bootstrap ==="
Write-Host "Setting up client for: $ServerName"
Write-Host ""

# Check we're in an SPT directory
if (-not (Test-Path "BepInEx")) {{
    Write-Host "ERROR: BepInEx\ directory not found." -ForegroundColor Red
    Write-Host "Run this script from your SPT installation directory."
    exit 1
}}

$FikaInstallerPath = Join-Path . "Fika-Installer.exe"
$TmpFile = Join-Path $env:TEMP "quma-mods-$([System.IO.Path]::GetRandomFileName()).zip"

try {{
    # Step 1: Install Fika via official installer
    Write-Host "Downloading Fika Installer..."
    Invoke-WebRequest -Uri $FikaInstallerUrl -OutFile $FikaInstallerPath -UseBasicParsing

    Write-Host "Installing Fika..."
    & $FikaInstallerPath install fika
    if ($LASTEXITCODE -ne 0) {{
        Write-Host "WARNING: Fika Installer returned exit code $LASTEXITCODE" -ForegroundColor Yellow
    }}

    # Step 2: Download additional mods (Convoy, etc.)
    Write-Host "Downloading additional mods..."
    Invoke-WebRequest -Uri $ArchiveUrl -OutFile $TmpFile -UseBasicParsing

    Write-Host "Extracting mods..."
    Expand-Archive -Path $TmpFile -DestinationPath . -Force

    # Configure launcher server address
    if (Test-Path $LauncherConfig) {{
        $cfg = Get-Content $LauncherConfig -Raw | ConvertFrom-Json
        if (-not $cfg.Server) {{
            $cfg | Add-Member -NotePropertyName 'Server' -NotePropertyValue ([PSCustomObject]@{{Url=$SptServerUrl}})
        }} else {{
            $cfg.Server | Add-Member -NotePropertyName 'Url' -NotePropertyValue $SptServerUrl -Force
        }}
        $cfg | ConvertTo-Json -Depth 10 | Set-Content $LauncherConfig -Encoding UTF8
        Write-Host "Launcher configured: server address set to $SptServerUrl" -ForegroundColor Green
    }} else {{
        Write-Host "NOTE: Launcher config not found at $LauncherConfig" -ForegroundColor Yellow
        Write-Host "  Launch SPT once, then re-run this script, or set the server address manually:"
        Write-Host "  $SptServerUrl"
    }}

    Write-Host ""
    Write-Host "=== Setup Complete ===" -ForegroundColor Green
    Write-Host ""
    Write-Host "Next steps:"
    Write-Host "  1. Launch SPT and connect to: $SptServerUrl"
}} finally {{
    if (Test-Path $TmpFile) {{ Remove-Item $TmpFile -Force }}
    if (Test-Path $FikaInstallerPath) {{ Remove-Item $FikaInstallerPath -Force }}
}}
"#
    )
}

pub(crate) fn build_mod_zip(
    spt_dir: &std::path::Path,
    files: &[crate::db::mods::InstalledFile],
) -> anyhow::Result<Vec<u8>> {
    use std::io::{Cursor, Write};
    use zip::write::SimpleFileOptions;
    use zip::ZipWriter;

    let buf = Cursor::new(Vec::new());
    let mut zip = ZipWriter::new(buf);
    let options = SimpleFileOptions::default().compression_method(zip::CompressionMethod::Deflated);

    for file in files {
        if std::path::Path::new(&file.file_path).is_absolute()
            || file.file_path.split('/').any(|c| c == "..")
            || file.file_path.split('\\').any(|c| c == "..")
        {
            tracing::warn!(path = %file.file_path, "skipping file with unsafe path in mod archive");
            continue;
        }

        let full_path = spt_dir.join(&file.file_path);
        match std::fs::read(&full_path) {
            Ok(data) => {
                zip.start_file(&file.file_path, options)?;
                zip.write_all(&data)?;
            }
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                tracing::warn!(path = %file.file_path, "skipping missing file in mod archive");
                continue;
            }
            Err(e) => return Err(e.into()),
        }
    }

    let cursor = zip.finish()?;
    Ok(cursor.into_inner())
}

#[cfg(test)]
mod tests {
    use super::*;

    const TEST_SPT_URL: &str = "https://example.com:6969";

    #[test]
    fn bash_script_escapes_shell_metacharacters() {
        let script = generate_bash_script(
            "My'; rm -rf /; echo 'Server",
            "https://example.com/quma/join/mods.zip?code=code1",
            TEST_SPT_URL,
        );
        assert!(script.contains("My'\\''"));
        assert!(script.contains("SERVER_NAME='My'\\''"));
        let server_name_line = script
            .lines()
            .find(|line| line.starts_with("SERVER_NAME="))
            .expect("SERVER_NAME line should exist");
        assert!(server_name_line.starts_with("SERVER_NAME='My'\\''"));
    }

    #[test]
    fn bash_script_escapes_single_quotes_in_url() {
        let script = generate_bash_script(
            "Server",
            "https://example.com'injected/quma/join/mods.zip?code=code1",
            TEST_SPT_URL,
        );
        assert!(script.contains("example.com'\\''injected"));
    }

    #[test]
    fn bash_script_uses_fika_installer() {
        let script = generate_bash_script(
            "Server",
            "https://example.com/quma/join/mods.zip?code=code1",
            TEST_SPT_URL,
        );
        assert!(script.contains("FIKA_INSTALLER_URL="));
        assert!(script.contains("Fika-Installer.exe"));
        assert!(script.contains("wine Fika-Installer.exe install fika"));
        assert!(
            !script.contains("|| echo"),
            "wine failure should not be swallowed"
        );
    }

    #[test]
    fn bash_script_uses_spt_server_url_for_launcher() {
        let script = generate_bash_script(
            "Server",
            "https://example.com/quma/join/mods.zip?code=code1",
            "https://example.com:6969",
        );
        assert!(script.contains("SPT_SERVER_URL='https://example.com:6969'"));
        assert!(script.contains("server address set to $SPT_SERVER_URL"));
    }

    #[test]
    fn bash_script_uses_curl_fail_flag() {
        let script = generate_bash_script(
            "Server",
            "https://example.com/quma/join/mods.zip?code=code1",
            TEST_SPT_URL,
        );
        assert!(script.contains("curl -fsSL"), "curl should use --fail flag");
        assert!(
            !script.contains("curl -sSL "),
            "should not have curl without --fail"
        );
    }

    #[test]
    fn powershell_script_escapes_single_quotes() {
        let script = generate_powershell_script(
            "My' Server",
            "https://example.com/quma/join/mods.zip?code=code1",
            TEST_SPT_URL,
        );
        assert!(script.contains("My'' Server"));
    }

    #[test]
    fn powershell_script_escapes_single_quotes_in_url() {
        let script = generate_powershell_script(
            "Server",
            "https://example.com'injected/quma/join/mods.zip?code=code1",
            TEST_SPT_URL,
        );
        assert!(script.contains("example.com''injected"));
    }

    #[test]
    fn powershell_script_uses_fika_installer() {
        let script = generate_powershell_script(
            "Server",
            "https://example.com/quma/join/mods.zip?code=code1",
            TEST_SPT_URL,
        );
        assert!(script.contains("$FikaInstallerUrl ="));
        assert!(script.contains("Fika-Installer.exe"));
        assert!(script.contains("install fika"));
    }

    #[test]
    fn powershell_script_uses_spt_server_url_for_launcher() {
        let script = generate_powershell_script(
            "Server",
            "https://example.com/quma/join/mods.zip?code=code1",
            "https://example.com:6969",
        );
        assert!(script.contains("$SptServerUrl = 'https://example.com:6969'"));
        assert!(script.contains("server address set to $SptServerUrl"));
    }

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

        let zip_bytes = build_mod_zip(spt_dir.path(), &files).unwrap();
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

        let zip_bytes = build_mod_zip(spt_dir.path(), &files).unwrap();
        let reader = zip::ZipArchive::new(std::io::Cursor::new(zip_bytes)).unwrap();
        assert_eq!(reader.len(), 0, "absolute path should be skipped");
    }
}
