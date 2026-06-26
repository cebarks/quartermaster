use actix_web::{web, HttpResponse};
use askama::Template;

use crate::config::{FIKA_CLIENT_FORGE_ID, NARCONET_FORGE_MOD_ID};
use crate::web::error::WebError;
use crate::web::invite::validate_invite_code;
use crate::web::state::AppState;

const DEFAULT_SERVER_NAME: &str = "SPT Server";

const BOOTSTRAP_FORGE_IDS: &[i64] = &[
    NARCONET_FORGE_MOD_ID, // 2441
    FIKA_CLIENT_FORGE_ID,  // 2326
];

#[derive(Debug, serde::Deserialize)]
pub struct JoinQuery {
    pub code: Option<String>,
}

#[derive(Template)]
#[template(path = "join.html")]
struct JoinTemplate {
    server_name: String,
    spt_version: String,
    external_url: String,
    fika_installed: bool,
    modsync_installed: bool,
    mod_count: usize,
    code: String,
    error: Option<String>,
}

fn referrer_policy(resp: HttpResponse) -> HttpResponse {
    let mut resp = resp;
    resp.headers_mut().insert(
        actix_web::http::header::HeaderName::from_static("referrer-policy"),
        actix_web::http::header::HeaderValue::from_static("no-referrer"),
    );
    resp
}

pub async fn join_page(
    query: web::Query<JoinQuery>,
    state: web::Data<AppState>,
) -> actix_web::Result<HttpResponse> {
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
            external_url: String::new(),
            fika_installed: state.fika_installed,
            modsync_installed: state
                .modsync_installed
                .load(std::sync::atomic::Ordering::Relaxed),
            mod_count: 0,
            code,
            error: Some(e.to_string()),
        };
        return Ok(referrer_policy(
            HttpResponse::BadRequest()
                .content_type("text/html")
                .body(tmpl.render().map_err(WebError::from)?),
        ));
    }

    let (external_url, server_name) = {
        let config = state.config.read();
        let external_url = match &config.external_url {
            Some(url) => url.clone(),
            None => {
                return Ok(referrer_policy(
                    HttpResponse::ServiceUnavailable()
                        .content_type("text/html")
                        .body(
                            "Bootstrap not configured: external_url is required in quartermaster.toml",
                        ),
                ));
            }
        };

        let server_name = config
            .server_name
            .clone()
            .unwrap_or_else(|| DEFAULT_SERVER_NAME.to_string());
        (external_url, server_name)
    };

    let modsync_installed = state
        .modsync_installed
        .load(std::sync::atomic::Ordering::Relaxed);

    // Count client-syncable mods (mods with BepInEx/ files, excluding infrastructure)
    let db = state.db.clone();
    let mod_count = web::block(move || {
        let db = db.lock();
        db.count_client_syncable_mods()
    })
    .await
    .map_err(WebError::from)?
    .map_err(WebError::from)?;

    let tmpl = JoinTemplate {
        server_name,
        spt_version: state.spt_info.spt_version.clone(),
        external_url,
        fika_installed: state.fika_installed,
        modsync_installed,
        mod_count,
        code,
        error: None,
    };

    Ok(referrer_policy(
        HttpResponse::Ok()
            .content_type("text/html")
            .body(tmpl.render().map_err(WebError::from)?),
    ))
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
                .body("No bootstrap mods (NarcoNet/Fika) are installed on this server"),
        ));
    }

    // Build ZIP archive in memory
    let spt_dir = state.spt_dir.clone();
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
        Some(url) => url.clone(),
        None => {
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

    let script = generate_bash_script(&server_name, &external_url, &code);

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
        Some(url) => url.clone(),
        None => {
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

    let script = generate_powershell_script(&server_name, &external_url, &code);

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

fn escape_bash(s: &str) -> String {
    s.replace('\'', "'\\''")
}

fn escape_powershell(s: &str) -> String {
    s.replace('\'', "''")
}

fn generate_bash_script(server_name: &str, external_url: &str, code: &str) -> String {
    let server_name = escape_bash(server_name);
    let external_url = escape_bash(external_url.trim_end_matches('/'));
    let code = escape_bash(code);

    format!(
        r#"#!/usr/bin/env bash
set -euo pipefail

SERVER_NAME='{server_name}'
SERVER_URL='{external_url}'
ARCHIVE_URL='{external_url}/quma/join/mods.zip?code={code}'
LAUNCHER_CONFIG='SPT/user/launcher/config.json'

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
for cmd in curl unzip; do
    if ! command -v "$cmd" &>/dev/null; then
        echo "ERROR: '$cmd' is required but not installed."
        exit 1
    fi
done

TMPFILE=$(mktemp /tmp/quma-mods-XXXXXX.zip)
trap 'rm -f "$TMPFILE"' EXIT

echo "Downloading mod archive..."
curl -sSL -o "$TMPFILE" "$ARCHIVE_URL"

echo "Extracting mods..."
unzip -o "$TMPFILE" -d .

# Configure launcher server address
if [ -f "$LAUNCHER_CONFIG" ]; then
    if command -v python3 &>/dev/null; then
        python3 -c "
import json, sys
with open(sys.argv[1]) as f: cfg = json.load(f)
cfg.setdefault('Server', {{}})['Url'] = sys.argv[2]
with open(sys.argv[1], 'w') as f: json.dump(cfg, f, indent=2)
" "$LAUNCHER_CONFIG" "$SERVER_URL"
        echo "Launcher configured: server address set to $SERVER_URL"
    else
        echo "NOTE: python3 not found — set the server address manually in SPT Launcher:"
        echo "  $SERVER_URL"
    fi
else
    echo "NOTE: Launcher config not found at $LAUNCHER_CONFIG"
    echo "  Launch SPT once, then re-run this script, or set the server address manually:"
    echo "  $SERVER_URL"
fi

echo ""
echo "=== Setup Complete ==="
echo ""
echo "Next steps:"
echo "  1. Launch SPT and connect"
echo "  2. After connecting, register at: $SERVER_URL/quma/register?code={code}"
"#
    )
}

fn generate_powershell_script(server_name: &str, external_url: &str, code: &str) -> String {
    let server_name = escape_powershell(server_name);
    let external_url = escape_powershell(external_url.trim_end_matches('/'));
    let code = escape_powershell(code);

    format!(
        r#"$ErrorActionPreference = 'Stop'

$ServerName = '{server_name}'
$ServerUrl = '{external_url}'
$ArchiveUrl = '{external_url}/quma/join/mods.zip?code={code}'
$LauncherConfig = 'SPT\user\launcher\config.json'

Write-Host "=== Quartermaster Bootstrap ==="
Write-Host "Setting up client for: $ServerName"
Write-Host ""

# Check we're in an SPT directory
if (-not (Test-Path "BepInEx")) {{
    Write-Host "ERROR: BepInEx\ directory not found." -ForegroundColor Red
    Write-Host "Run this script from your SPT installation directory."
    exit 1
}}

$TmpFile = Join-Path $env:TEMP "quma-mods-$([System.IO.Path]::GetRandomFileName()).zip"

try {{
    Write-Host "Downloading mod archive..."
    Invoke-WebRequest -Uri $ArchiveUrl -OutFile $TmpFile -UseBasicParsing

    Write-Host "Extracting mods..."
    Expand-Archive -Path $TmpFile -DestinationPath . -Force

    # Configure launcher server address
    if (Test-Path $LauncherConfig) {{
        $cfg = Get-Content $LauncherConfig -Raw | ConvertFrom-Json
        if (-not $cfg.Server) {{
            $cfg | Add-Member -NotePropertyName 'Server' -NotePropertyValue ([PSCustomObject]@{{}})
        }}
        $cfg.Server.Url = $ServerUrl
        $cfg | ConvertTo-Json -Depth 10 | Set-Content $LauncherConfig -Encoding UTF8
        Write-Host "Launcher configured: server address set to $ServerUrl" -ForegroundColor Green
    }} else {{
        Write-Host "NOTE: Launcher config not found at $LauncherConfig" -ForegroundColor Yellow
        Write-Host "  Launch SPT once, then re-run this script, or set the server address manually:"
        Write-Host "  $ServerUrl"
    }}

    Write-Host ""
    Write-Host "=== Setup Complete ===" -ForegroundColor Green
    Write-Host ""
    Write-Host "Next steps:"
    Write-Host "  1. Launch SPT and connect"
    Write-Host "  2. After connecting, register at: $ServerUrl/quma/register?code={code}"
}} finally {{
    if (Test-Path $TmpFile) {{ Remove-Item $TmpFile -Force }}
}}
"#
    )
}

fn build_mod_zip(
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

    #[test]
    fn bash_script_escapes_shell_metacharacters() {
        let script = generate_bash_script(
            "My'; rm -rf /; echo 'Server",
            "https://example.com",
            "code1",
        );
        // Verify the quote is escaped — the SERVER_NAME line should contain the escaped form
        assert!(script.contains("My'\\''"));
        // Verify the entire string is properly escaped
        assert!(script.contains("SERVER_NAME='My'\\''"));
        // Verify the semicolon and subsequent commands are part of the escaped string,
        // not executable shell code
        let server_name_line = script
            .lines()
            .find(|line| line.starts_with("SERVER_NAME="))
            .expect("SERVER_NAME line should exist");
        // The line should start with SERVER_NAME=' and contain the escaped quote sequence
        assert!(server_name_line.starts_with("SERVER_NAME='My'\\''"));
    }

    #[test]
    fn bash_script_escapes_single_quotes_in_url() {
        let script = generate_bash_script("Server", "https://example.com'injected", "code1");
        assert!(script.contains("example.com'\\''injected"));
    }

    #[test]
    fn powershell_script_escapes_single_quotes() {
        let script = generate_powershell_script("My' Server", "https://example.com", "code1");
        assert!(script.contains("My'' Server"));
    }

    #[test]
    fn powershell_script_escapes_single_quotes_in_url() {
        let script = generate_powershell_script("Server", "https://example.com'injected", "code1");
        assert!(script.contains("example.com''injected"));
    }
}
