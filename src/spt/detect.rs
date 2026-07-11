use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use serde::Deserialize;

/// Metadata extracted from a validated SPT installation.
#[derive(Debug, Clone)]
pub struct SptInfo {
    #[allow(dead_code)] // populated during detection, used in tests
    pub root: PathBuf,
    pub spt_version: String,
    pub tarkov_version: String,
}

/// Internal deserialization target for SPT/SPT_Data/configs/core.json.
#[derive(Deserialize)]
struct CoreJson {
    #[serde(alias = "compatibleTarkovVersion")]
    compatible_tarkov_version: Option<String>,
}

/// Internal deserialization target for SPT/SPT_Data/configs/http.json.
#[derive(Deserialize)]
struct HttpJson {
    ip: Option<String>,
    port: Option<u16>,
}

/// Internal deserialization target for SPT/SPT.Server.deps.json.
#[derive(Deserialize)]
struct DepsJson {
    libraries: Option<serde_json::Map<String, serde_json::Value>>,
}

/// Required markers that identify a valid SPT 4.0+ installation directory.
const REQUIRED_PATHS: &[&str] = &[
    "SPT/SPT.Server.exe",
    "SPT/SPT_Data/configs/core.json",
    "SPT/user/mods",
    "BepInEx/plugins",
];

/// Validate that `path` contains the expected SPT directory structure.
///
/// Checks for the presence of SPT/SPT.Server.exe, the server config directory,
/// the user mods directory, and BepInEx plugins directory.
pub fn validate_spt_dir(path: &Path) -> Result<()> {
    for entry in REQUIRED_PATHS {
        let full = path.join(entry);
        if !full.exists() {
            anyhow::bail!("not a valid SPT 4.0+ install: missing {entry}");
        }
    }
    Ok(())
}

/// Parse SPT version from `SPT.Server.deps.json` and Tarkov version from `core.json`.
pub fn read_spt_version(spt_dir: &Path) -> Result<SptInfo> {
    // 1. Read Tarkov version from core.json
    let core_path = spt_dir.join("SPT/SPT_Data/configs/core.json");
    let contents = std::fs::read_to_string(&core_path)
        .with_context(|| format!("failed to read {}", core_path.display()))?;

    let parsed: CoreJson = serde_json::from_str(&contents)
        .with_context(|| format!("failed to parse {}", core_path.display()))?;

    let tarkov_version = parsed.compatible_tarkov_version.ok_or_else(|| {
        anyhow::anyhow!(
            "not a valid SPT 4.0+ install: core.json missing compatibleTarkovVersion field"
        )
    })?;

    // 2. Read SPT version from SPT.Server.deps.json
    let spt_version = read_spt_version_from_deps(spt_dir)?;

    Ok(SptInfo {
        root: spt_dir.to_path_buf(),
        spt_version,
        tarkov_version,
    })
}

/// Parse SPT version from `SPT/SPT.Server.deps.json`.
/// Looks for a key like `"SPT.Server/4.0.13-RELEASE+..."` and extracts `4.0.13`.
fn read_spt_version_from_deps(spt_dir: &Path) -> Result<String> {
    let deps_path = spt_dir.join("SPT/SPT.Server.deps.json");
    let contents = std::fs::read_to_string(&deps_path)
        .with_context(|| format!("failed to read {}", deps_path.display()))?;
    let parsed: DepsJson = serde_json::from_str(&contents)
        .with_context(|| format!("failed to parse {}", deps_path.display()))?;

    if let Some(libs) = parsed.libraries {
        for key in libs.keys() {
            if let Some(version_part) = key.strip_prefix("SPT.Server/") {
                // Extract version before first dash: "4.0.13-RELEASE+..." -> "4.0.13"
                let version = version_part.split('-').next().unwrap_or(version_part);
                return Ok(version.to_string());
            }
        }
    }

    anyhow::bail!(
        "could not find SPT.Server version in {}",
        deps_path.display()
    )
}

/// Detect the SPT installation directory using the following priority:
///
/// 1. An explicit path provided by the caller (e.g. `--spt-dir`)
/// 2. The `QUMA_SPT_DIR` environment variable
/// 3. Walking up from `cwd` (or the process CWD) looking for the SPT root
///
/// Returns an error if none of the strategies succeed.
/// Parse `http.json` from the SPT installation for the server's IP and port.
///
/// Returns `None` on any failure (missing file, bad JSON, missing fields).
pub fn read_http_config(spt_dir: &Path) -> Option<(String, u16)> {
    let http_path = spt_dir.join("SPT/SPT_Data/configs/http.json");
    let contents = std::fs::read_to_string(&http_path).ok()?;
    let parsed: HttpJson = serde_json::from_str(&contents).ok()?;
    Some((parsed.ip?, parsed.port?))
}

#[cfg(test)]
#[allow(deprecated)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    /// Create a fake SPT directory with the minimum required structure and a
    /// realistic core.json file. Returns the path to the SPT root inside `base`.
    fn create_fake_spt_dir(base: &Path) -> PathBuf {
        let spt_root = base.to_path_buf();

        // SPT/SPT.Server.exe
        std::fs::create_dir_all(spt_root.join("SPT")).unwrap();
        std::fs::write(spt_root.join("SPT/SPT.Server.exe"), b"").unwrap();

        // SPT/SPT_Data/configs/core.json
        let configs_dir = spt_root.join("SPT/SPT_Data/configs");
        std::fs::create_dir_all(&configs_dir).unwrap();
        std::fs::write(
            configs_dir.join("core.json"),
            r#"{"compatibleTarkovVersion": "0.16.9-40087"}"#,
        )
        .unwrap();

        // SPT/SPT.Server.deps.json (for version detection)
        std::fs::write(
            spt_root.join("SPT/SPT.Server.deps.json"),
            r#"{"libraries":{"SPT.Server/4.0.13-RELEASE+abc123.20260101":{}}}"#,
        )
        .unwrap();

        // SPT/user/mods/
        std::fs::create_dir_all(spt_root.join("SPT/user/mods")).unwrap();

        // BepInEx/plugins/
        std::fs::create_dir_all(spt_root.join("BepInEx/plugins")).unwrap();

        spt_root
    }

    #[test]
    fn validate_valid_spt_dir() {
        let tmp = TempDir::new().unwrap();
        let spt = create_fake_spt_dir(tmp.path());
        assert!(validate_spt_dir(&spt).is_ok());
    }

    #[test]
    fn validate_rejects_empty_dir() {
        let tmp = TempDir::new().unwrap();
        let err = validate_spt_dir(tmp.path()).unwrap_err();
        assert!(err.to_string().contains("missing SPT/SPT.Server.exe"));
    }

    #[test]
    fn validate_rejects_partial_dir() {
        let tmp = TempDir::new().unwrap();
        // Only create the exe — remaining markers are absent.
        std::fs::create_dir_all(tmp.path().join("SPT")).unwrap();
        std::fs::write(tmp.path().join("SPT/SPT.Server.exe"), b"").unwrap();

        let err = validate_spt_dir(tmp.path()).unwrap_err();
        // Should fail on one of the missing dirs (core.json path).
        assert!(err.to_string().contains("missing"));
    }

    #[test]
    fn read_spt_version_from_core_json() {
        let tmp = TempDir::new().unwrap();
        let spt = create_fake_spt_dir(tmp.path());
        let info = read_spt_version(&spt).unwrap();

        assert_eq!(info.spt_version, "4.0.13");
        assert_eq!(info.tarkov_version, "0.16.9-40087");
        assert_eq!(info.root, spt);
    }

    #[test]
    fn read_spt_version_malformed_json() {
        let tmp = TempDir::new().unwrap();
        let spt = create_fake_spt_dir(tmp.path());

        // Overwrite core.json with garbage
        let core_path = spt.join("SPT/SPT_Data/configs/core.json");
        std::fs::write(&core_path, "not json at all {{{").unwrap();

        let err = read_spt_version(&spt).unwrap_err();
        assert!(err.to_string().contains("failed to parse"));
    }

    #[test]
    fn read_spt_version_missing_fields() {
        let tmp = TempDir::new().unwrap();
        let spt = create_fake_spt_dir(tmp.path());

        // Overwrite core.json with valid JSON but missing required fields
        let core_path = spt.join("SPT/SPT_Data/configs/core.json");
        std::fs::write(&core_path, r#"{"someOtherField": true}"#).unwrap();

        let err = read_spt_version(&spt).unwrap_err();
        assert!(err.to_string().contains("missing"));
    }

    #[test]
    fn detect_from_explicit_path() {
        let tmp = TempDir::new().unwrap();
        let spt = create_fake_spt_dir(tmp.path());

        let dirs = crate::dirs::QumaDirs::detect(Some(&spt), None).unwrap();
        assert_eq!(dirs.spt_server, spt);
    }

    #[test]
    fn detect_from_cwd_walkup() {
        let tmp = TempDir::new().unwrap();
        let spt = create_fake_spt_dir(tmp.path());

        // Simulate being inside SPT/user/mods/ — walkup should find the root.
        let deep = spt.join("SPT/user/mods");
        let dirs = crate::dirs::QumaDirs::detect(None, Some(&deep)).unwrap();
        assert_eq!(dirs.spt_server, spt);
    }

    #[test]
    fn detect_fails_when_not_found() {
        temp_env::with_vars_unset(["QUMA_SPT_DIR", "QUMA_DIR"], || {
            let tmp = TempDir::new().unwrap();
            let err = crate::dirs::QumaDirs::detect(None, Some(tmp.path())).unwrap_err();
            assert!(
                err.to_string().contains("not found"),
                "unexpected error: {err}"
            );
        });
    }

    #[test]
    fn read_http_config_parses() {
        let tmp = TempDir::new().unwrap();
        let spt = create_fake_spt_dir(tmp.path());

        let http_path = spt.join("SPT/SPT_Data/configs/http.json");
        std::fs::write(&http_path, r#"{"ip": "127.0.0.1", "port": 6969}"#).unwrap();

        let (ip, port) = read_http_config(&spt).unwrap();
        assert_eq!(ip, "127.0.0.1");
        assert_eq!(port, 6969);
    }

    #[test]
    fn read_http_config_returns_none_when_missing() {
        let tmp = TempDir::new().unwrap();
        let spt = create_fake_spt_dir(tmp.path());
        // No http.json was created by the helper
        assert!(read_http_config(&spt).is_none());
    }

    #[test]
    fn detect_errors_on_invalid_quma_spt_dir() {
        let tmp = TempDir::new().unwrap();
        let bad_dir = tmp.path().join("nonexistent");

        temp_env::with_vars(
            [
                ("QUMA_SPT_DIR", Some(bad_dir.to_str().unwrap())),
                ("QUMA_DIR", None),
            ],
            || {
                let result = crate::dirs::QumaDirs::detect(None, Some(tmp.path()));
                assert!(result.is_err(), "should error when QUMA_SPT_DIR is invalid");
                let err_msg = format!("{:#}", result.unwrap_err());
                assert!(
                    err_msg.contains("QUMA_SPT_DIR"),
                    "error should mention QUMA_SPT_DIR: {err_msg}"
                );
            },
        );
    }
}
