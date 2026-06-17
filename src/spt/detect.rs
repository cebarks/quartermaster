use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use serde::Deserialize;

use crate::error::QumaError;

/// Metadata extracted from a validated SPT installation.
#[derive(Debug, Clone)]
pub struct SptInfo {
    pub root: PathBuf,
    pub spt_version: String,
    pub tarkov_version: String,
}

/// Internal deserialization target for SPT_Data/Server/configs/core.json.
#[derive(Deserialize)]
struct CoreJson {
    #[serde(alias = "sptVersion")]
    spt_version: Option<String>,

    #[serde(alias = "compatibleTarkovVersion")]
    compatible_tarkov_version: Option<String>,
}

/// Internal deserialization target for SPT_Data/Server/configs/http.json.
#[derive(Deserialize)]
struct HttpJson {
    ip: Option<String>,
    port: Option<u16>,
}

/// Required markers that identify a valid SPT 4.0+ installation directory.
const REQUIRED_PATHS: &[&str] = &[
    "SPT.Server.exe",
    "SPT_Data/Server/configs/core.json",
    "user/mods",
    "BepInEx/plugins",
];

/// Validate that `path` contains the expected SPT directory structure.
///
/// Checks for the presence of SPT.Server.exe, the server config directory,
/// the user mods directory, and BepInEx plugins directory.
pub fn validate_spt_dir(path: &Path) -> Result<()> {
    for entry in REQUIRED_PATHS {
        let full = path.join(entry);
        if !full.exists() {
            return Err(QumaError::InvalidSptDir(format!("missing {entry}")).into());
        }
    }
    Ok(())
}

/// Parse `core.json` from the SPT installation and extract version info.
pub fn read_spt_version(spt_dir: &Path) -> Result<SptInfo> {
    let core_path = spt_dir.join("SPT_Data/Server/configs/core.json");
    let contents = std::fs::read_to_string(&core_path)
        .with_context(|| format!("failed to read {}", core_path.display()))?;

    let parsed: CoreJson = serde_json::from_str(&contents)
        .with_context(|| format!("failed to parse {}", core_path.display()))?;

    let spt_version = parsed
        .spt_version
        .ok_or_else(|| QumaError::InvalidSptDir("core.json missing sptVersion field".into()))?;

    let tarkov_version = parsed.compatible_tarkov_version.ok_or_else(|| {
        QumaError::InvalidSptDir("core.json missing compatibleTarkovVersion field".into())
    })?;

    Ok(SptInfo {
        root: spt_dir.to_path_buf(),
        spt_version,
        tarkov_version,
    })
}

/// Detect the SPT installation directory using the following priority:
///
/// 1. An explicit path provided by the caller (e.g. `--spt-dir`)
/// 2. The `QUMA_SPT_DIR` environment variable
/// 3. Walking up from `cwd` (or the process CWD) looking for `SPT.Server.exe`
///
/// Returns `QumaError::SptDirNotFound` if none of the strategies succeed.
pub fn detect_spt_dir(explicit: Option<&Path>, cwd: Option<&Path>) -> Result<PathBuf> {
    // 1. Explicit path
    if let Some(p) = explicit {
        validate_spt_dir(p)?;
        return Ok(p.to_path_buf());
    }

    // 2. QUMA_SPT_DIR env
    if let Ok(env_val) = std::env::var("QUMA_SPT_DIR") {
        let env_path = PathBuf::from(&env_val);
        if validate_spt_dir(&env_path).is_ok() {
            return Ok(env_path);
        }
    }

    // 3. Walk up from cwd
    let start = match cwd {
        Some(p) => p.to_path_buf(),
        None => std::env::current_dir().context("failed to determine current directory")?,
    };

    let mut candidate = Some(start.as_path());
    while let Some(dir) = candidate {
        if validate_spt_dir(dir).is_ok() {
            return Ok(dir.to_path_buf());
        }
        candidate = dir.parent();
    }

    Err(QumaError::SptDirNotFound.into())
}

/// Parse `http.json` from the SPT installation for the server's IP and port.
///
/// Returns `None` on any failure (missing file, bad JSON, missing fields).
pub fn read_http_config(spt_dir: &Path) -> Option<(String, u16)> {
    let http_path = spt_dir.join("SPT_Data/Server/configs/http.json");
    let contents = std::fs::read_to_string(&http_path).ok()?;
    let parsed: HttpJson = serde_json::from_str(&contents).ok()?;
    Some((parsed.ip?, parsed.port?))
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    /// Create a fake SPT directory with the minimum required structure and a
    /// realistic core.json file. Returns the path to the SPT root inside `base`.
    fn create_fake_spt_dir(base: &Path) -> PathBuf {
        let spt_root = base.to_path_buf();

        // SPT.Server.exe (empty file is enough for existence checks)
        std::fs::write(spt_root.join("SPT.Server.exe"), b"").unwrap();

        // SPT_Data/Server/configs/core.json
        let configs_dir = spt_root.join("SPT_Data/Server/configs");
        std::fs::create_dir_all(&configs_dir).unwrap();
        std::fs::write(
            configs_dir.join("core.json"),
            r#"{"sptVersion": "4.0.13", "compatibleTarkovVersion": "0.16.9-40087"}"#,
        )
        .unwrap();

        // user/mods/
        std::fs::create_dir_all(spt_root.join("user/mods")).unwrap();

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
        assert!(err.to_string().contains("missing SPT.Server.exe"));
    }

    #[test]
    fn validate_rejects_partial_dir() {
        let tmp = TempDir::new().unwrap();
        // Only create the exe — remaining markers are absent.
        std::fs::write(tmp.path().join("SPT.Server.exe"), b"").unwrap();

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
        let core_path = spt.join("SPT_Data/Server/configs/core.json");
        std::fs::write(&core_path, "not json at all {{{").unwrap();

        let err = read_spt_version(&spt).unwrap_err();
        assert!(err.to_string().contains("failed to parse"));
    }

    #[test]
    fn read_spt_version_missing_fields() {
        let tmp = TempDir::new().unwrap();
        let spt = create_fake_spt_dir(tmp.path());

        // Overwrite core.json with valid JSON but missing required fields
        let core_path = spt.join("SPT_Data/Server/configs/core.json");
        std::fs::write(&core_path, r#"{"someOtherField": true}"#).unwrap();

        let err = read_spt_version(&spt).unwrap_err();
        assert!(err.to_string().contains("missing"));
    }

    #[test]
    fn detect_from_explicit_path() {
        let tmp = TempDir::new().unwrap();
        let spt = create_fake_spt_dir(tmp.path());

        let result = detect_spt_dir(Some(&spt), None).unwrap();
        assert_eq!(result, spt);
    }

    #[test]
    fn detect_from_cwd_walkup() {
        let tmp = TempDir::new().unwrap();
        let spt = create_fake_spt_dir(tmp.path());

        // Simulate being inside user/mods/ — walkup should find the root.
        let deep = spt.join("user/mods");
        let result = detect_spt_dir(None, Some(&deep)).unwrap();
        assert_eq!(result, spt);
    }

    #[test]
    fn detect_fails_when_not_found() {
        let tmp = TempDir::new().unwrap();
        // Empty dir — no SPT markers anywhere up the tree.
        let err = detect_spt_dir(None, Some(tmp.path())).unwrap_err();
        let quma_err = err.downcast_ref::<QumaError>().unwrap();
        assert!(matches!(quma_err, QumaError::SptDirNotFound));
    }

    #[test]
    fn read_http_config_parses() {
        let tmp = TempDir::new().unwrap();
        let spt = create_fake_spt_dir(tmp.path());

        let http_path = spt.join("SPT_Data/Server/configs/http.json");
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
}
