//! Generate client bootstrap downloads from one instance-specific data contract.
//!
//! Data is base64-encoded JSON, never interpolated as shell/Python/PowerShell
//! source. The platform implementations are kept as readable embedded files.
use base64::Engine as _;
use serde::Serialize;

use crate::web::error::WebError;
use crate::web::state::AppState;

#[derive(Debug, Serialize)]
pub(crate) struct BootstrapConfig {
    server_name: String,
    archive_url: String,
    server_url: String,
    spt_version: String,
    tarkov_version: String,
    required_files: Vec<&'static str>,
}

pub(crate) async fn for_instance(
    state: &AppState,
    server_name: String,
    archive_url: String,
    server_url: String,
) -> Result<BootstrapConfig, WebError> {
    let db = state.db.clone();
    let convoy_installed = actix_web::web::block(move || db.lock().is_forge_mod_installed(2806))
        .await
        .map_err(WebError::from)?
        .map_err(WebError::from)?;
    let mut required_files = Vec::new();
    if state.fika_installed {
        required_files.push("BepInEx/plugins/Fika/Fika.Core.dll");
    }
    if convoy_installed {
        required_files.extend([
            "BepInEx/plugins/Convoy/Convoy.dll",
            "BepInEx/plugins/Convoy/System.IO.Hashing.dll",
        ]);
    }
    Ok(BootstrapConfig {
        server_name,
        archive_url,
        server_url,
        spt_version: state.spt_info.spt_version.clone(),
        tarkov_version: state.spt_info.tarkov_version.clone(),
        required_files,
    })
}

fn encoded_config(config: &BootstrapConfig) -> String {
    // All fields serialize infallibly: strings and a vector of string literals.
    let json = serde_json::to_vec(config).expect("bootstrap strings serialize to JSON");
    base64::engine::general_purpose::STANDARD.encode(json)
}

pub(crate) fn generate_bash_script(config: &BootstrapConfig) -> String {
    let python = include_str!("bootstrap/client.py")
        .replace("__QUMA_CONFIG_BASE64__", &encoded_config(config));
    include_str!("bootstrap/bootstrap.sh").replace("__QUMA_PYTHON__", &python)
}

pub(crate) fn generate_powershell_script(config: &BootstrapConfig) -> String {
    include_str!("bootstrap/bootstrap.ps1")
        .replace("__QUMA_CONFIG_BASE64__", &encoded_config(config))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn config() -> BootstrapConfig {
        BootstrapConfig {
            server_name: "Untrusted '\nQUMA_BOOTSTRAP_PYTHON\n$(touch injected)".into(),
            archive_url: "https://example.test/quma/join/mods.zip?code=fixture".into(),
            server_url: "https://example.test".into(),
            spt_version: "4.1.6".into(),
            tarkov_version: "0.16.9.40743".into(),
            required_files: vec!["BepInEx/plugins/Fika/Fika.Core.dll"],
        }
    }

    #[test]
    fn instance_data_is_encoded_not_interpolated_as_program_source() {
        let config = config();
        let encoded = encoded_config(&config);
        let decoded = base64::engine::general_purpose::STANDARD
            .decode(&encoded)
            .expect("generated bootstrap config must be valid base64");
        let value: serde_json::Value = serde_json::from_slice(&decoded)
            .expect("generated bootstrap config must contain valid JSON");
        assert_eq!(value["server_name"], config.server_name);
        for script in [
            generate_bash_script(&config),
            generate_powershell_script(&config),
        ] {
            assert!(script.contains(&encoded));
            assert!(!script.contains(&config.server_name));
            assert!(!script.contains("__QUMA_"));
        }
    }

    #[test]
    fn templates_do_not_download_or_execute_an_external_installer() {
        for script in [
            generate_bash_script(&config()),
            generate_powershell_script(&config()),
        ] {
            assert!(!script.contains("Fika-Installer.exe"));
            assert!(!script.contains("releases/latest/download"));
        }
    }
}
