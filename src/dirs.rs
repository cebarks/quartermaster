use std::path::{Path, PathBuf};

use anyhow::{Context, Result};

#[derive(Debug, Clone)]
pub struct QumaDirs {
    pub root: PathBuf,
    pub spt_server: PathBuf,
    #[allow(dead_code)] // ponytail: used once converge.rs migrates to new layout
    pub headless: PathBuf,
    #[allow(dead_code)] // ponytail: used once converge.rs migrates to new layout
    pub overlay: PathBuf,
    legacy: bool,
}

impl QumaDirs {
    pub fn from_root(root: PathBuf) -> Self {
        Self {
            spt_server: root.join("spt-server"),
            headless: root.join("headless"),
            overlay: root.join("headless-overlay"),
            root,
            legacy: false,
        }
    }

    pub fn from_legacy(spt_dir: PathBuf) -> Self {
        Self {
            spt_server: spt_dir.clone(),
            headless: PathBuf::new(),
            overlay: PathBuf::new(),
            root: spt_dir,
            legacy: true,
        }
    }

    pub fn is_legacy(&self) -> bool {
        self.legacy
    }

    pub fn detect(explicit: Option<&Path>, cwd: Option<&Path>) -> Result<Self> {
        // 1. Explicit path
        if let Some(p) = explicit {
            return Self::classify_and_build(p);
        }

        // 2. QUMA_DIR env var (new)
        if let Ok(env_val) = std::env::var("QUMA_DIR") {
            let env_path = PathBuf::from(&env_val);
            return Self::classify_and_build(&env_path)
                .with_context(|| format!("QUMA_DIR={env_val} is not a valid quma directory"));
        }

        // 3. QUMA_SPT_DIR env var (deprecated)
        if let Ok(env_val) = std::env::var("QUMA_SPT_DIR") {
            tracing::warn!("QUMA_SPT_DIR is deprecated, use QUMA_DIR instead");
            let env_path = PathBuf::from(&env_val);
            crate::spt::detect::validate_spt_dir(&env_path)
                .with_context(|| format!("QUMA_SPT_DIR={env_val} is not a valid SPT directory"))?;
            return Ok(Self::from_legacy(env_path));
        }

        // 4. Walk up from cwd
        let start = match cwd {
            Some(p) => p.to_path_buf(),
            None => std::env::current_dir().context("failed to determine current directory")?,
        };

        let mut candidate = Some(start.as_path());
        while let Some(dir) = candidate {
            if let Ok(dirs) = Self::classify_and_build(dir) {
                return Ok(dirs);
            }
            candidate = dir.parent();
        }

        anyhow::bail!("Quartermaster directory not found — run `quma setup` or pass --quma-dir")
    }

    fn classify_and_build(path: &Path) -> Result<Self> {
        // New layout: quartermaster.toml or .db at root, spt-server/ subdir with SPT markers
        let has_config = path.join("quartermaster.toml").exists();
        let has_db = path.join("quartermaster.db").exists();
        let spt_subdir = path.join("spt-server");

        if (has_config || has_db) && spt_subdir.exists() {
            if crate::spt::detect::validate_spt_dir(&spt_subdir).is_ok() {
                return Ok(Self::from_root(path.to_path_buf()));
            }
            // spt-server/ exists but isn't valid yet (fresh setup before first run)
            if has_config || has_db {
                return Ok(Self::from_root(path.to_path_buf()));
            }
        }

        // Legacy layout: SPT markers at root level
        if crate::spt::detect::validate_spt_dir(path).is_ok() {
            tracing::warn!(
                "Detected legacy directory layout. Run `quma migrate` to update to the new layout."
            );
            return Ok(Self::from_legacy(path.to_path_buf()));
        }

        anyhow::bail!("not a valid quma or SPT directory: {}", path.display())
    }

    // -- Quma data paths --

    pub fn db_path(&self) -> PathBuf {
        self.root.join("quartermaster.db")
    }

    pub fn config_path(&self) -> PathBuf {
        self.root.join("quartermaster.toml")
    }

    pub fn staging_dir(&self) -> PathBuf {
        if self.legacy {
            self.root.join("quartermaster/.staging")
        } else {
            self.root.join(".staging")
        }
    }

    pub fn disabled_dir(&self) -> PathBuf {
        if self.legacy {
            self.root.join("quartermaster/disabled")
        } else {
            self.root.join("disabled")
        }
    }

    pub fn config_history_dir(&self) -> PathBuf {
        if self.legacy {
            self.root.join("quartermaster/config-history")
        } else {
            self.root.join("config-history")
        }
    }

    pub fn queue_dir(&self) -> PathBuf {
        if self.legacy {
            self.root.join(".quartermaster/queued")
        } else {
            self.root.join("queued")
        }
    }

    #[allow(dead_code)] // ponytail: used once cache module migrates
    pub fn cache_dir(&self) -> PathBuf {
        if self.legacy {
            self.root.join("quartermaster-cache")
        } else {
            self.root.join("cache")
        }
    }

    #[allow(dead_code)] // ponytail: used once file logging migrates
    pub fn log_dir(&self) -> PathBuf {
        self.root.join("logs")
    }

    pub fn tls_cert(&self) -> PathBuf {
        self.root.join("quma-cert.pem")
    }

    pub fn tls_key(&self) -> PathBuf {
        self.root.join("quma-key.pem")
    }

    pub fn backup_dir(&self, relative: &str) -> PathBuf {
        self.root.join(relative)
    }

    // -- SPT server paths --

    pub fn mod_file_path(&self, rel: &str) -> PathBuf {
        self.spt_server.join(rel)
    }

    pub fn server_mods_dir(&self) -> PathBuf {
        self.spt_server.join("SPT/user/mods")
    }

    pub fn client_mods_dir(&self) -> PathBuf {
        self.spt_server.join("BepInEx/plugins")
    }

    pub fn profiles_dir(&self) -> PathBuf {
        self.spt_server.join("SPT/user/profiles")
    }

    #[allow(dead_code)] // ponytail: used once spt/detect migrates
    pub fn core_json(&self) -> PathBuf {
        self.spt_server.join("SPT/SPT_Data/configs/core.json")
    }

    #[allow(dead_code)] // ponytail: used once server_detect migrates fully
    pub fn http_json(&self) -> PathBuf {
        self.spt_server.join("SPT/SPT_Data/configs/http.json")
    }

    #[allow(dead_code)] // ponytail: used once fika module migrates
    pub fn fika_config(&self) -> PathBuf {
        self.spt_server
            .join("SPT/user/mods/fika-server/assets/configs/fika.jsonc")
    }

    // -- Headless paths --

    #[allow(dead_code)] // ponytail: used once converge.rs migrates to new layout
    pub fn client_overlay(&self, index: u32) -> PathBuf {
        self.overlay.join(format!("client-{index}"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    #[test]
    fn from_root_derives_all_paths() {
        let dirs = QumaDirs::from_root(PathBuf::from("/opt/quma"));
        assert_eq!(dirs.root, PathBuf::from("/opt/quma"));
        assert_eq!(dirs.spt_server, PathBuf::from("/opt/quma/spt-server"));
        assert_eq!(dirs.headless, PathBuf::from("/opt/quma/headless"));
        assert_eq!(dirs.overlay, PathBuf::from("/opt/quma/headless-overlay"));
    }

    #[test]
    fn data_paths_at_root() {
        let dirs = QumaDirs::from_root(PathBuf::from("/opt/quma"));
        assert_eq!(dirs.db_path(), PathBuf::from("/opt/quma/quartermaster.db"));
        assert_eq!(
            dirs.config_path(),
            PathBuf::from("/opt/quma/quartermaster.toml")
        );
        assert_eq!(dirs.staging_dir(), PathBuf::from("/opt/quma/.staging"));
        assert_eq!(dirs.disabled_dir(), PathBuf::from("/opt/quma/disabled"));
        assert_eq!(
            dirs.config_history_dir(),
            PathBuf::from("/opt/quma/config-history")
        );
        assert_eq!(dirs.queue_dir(), PathBuf::from("/opt/quma/queued"));
        assert_eq!(dirs.cache_dir(), PathBuf::from("/opt/quma/cache"));
        assert_eq!(dirs.log_dir(), PathBuf::from("/opt/quma/logs"));
        assert_eq!(dirs.tls_cert(), PathBuf::from("/opt/quma/quma-cert.pem"));
        assert_eq!(dirs.tls_key(), PathBuf::from("/opt/quma/quma-key.pem"));
    }

    #[test]
    fn spt_server_paths() {
        let dirs = QumaDirs::from_root(PathBuf::from("/opt/quma"));
        assert_eq!(
            dirs.server_mods_dir(),
            PathBuf::from("/opt/quma/spt-server/SPT/user/mods")
        );
        assert_eq!(
            dirs.client_mods_dir(),
            PathBuf::from("/opt/quma/spt-server/BepInEx/plugins")
        );
        assert_eq!(
            dirs.profiles_dir(),
            PathBuf::from("/opt/quma/spt-server/SPT/user/profiles")
        );
        assert_eq!(
            dirs.mod_file_path("SPT/user/mods/test"),
            PathBuf::from("/opt/quma/spt-server/SPT/user/mods/test")
        );
    }

    #[test]
    fn headless_overlay_paths() {
        let dirs = QumaDirs::from_root(PathBuf::from("/opt/quma"));
        assert_eq!(
            dirs.client_overlay(0),
            PathBuf::from("/opt/quma/headless-overlay/client-0")
        );
        assert_eq!(
            dirs.client_overlay(3),
            PathBuf::from("/opt/quma/headless-overlay/client-3")
        );
    }

    #[test]
    fn legacy_mode_points_spt_server_at_root() {
        let dirs = QumaDirs::from_legacy(PathBuf::from("/home/user/spt-server"));
        assert_eq!(dirs.root, PathBuf::from("/home/user/spt-server"));
        assert_eq!(dirs.spt_server, PathBuf::from("/home/user/spt-server"));
        assert_eq!(
            dirs.db_path(),
            PathBuf::from("/home/user/spt-server/quartermaster.db")
        );
        assert_eq!(
            dirs.server_mods_dir(),
            PathBuf::from("/home/user/spt-server/SPT/user/mods")
        );
    }

    #[test]
    fn legacy_data_paths_match_old_layout() {
        let dirs = QumaDirs::from_legacy(PathBuf::from("/home/user/spt-server"));
        assert_eq!(
            dirs.staging_dir(),
            PathBuf::from("/home/user/spt-server/quartermaster/.staging")
        );
        assert_eq!(
            dirs.disabled_dir(),
            PathBuf::from("/home/user/spt-server/quartermaster/disabled")
        );
        assert_eq!(
            dirs.config_history_dir(),
            PathBuf::from("/home/user/spt-server/quartermaster/config-history")
        );
        assert_eq!(
            dirs.queue_dir(),
            PathBuf::from("/home/user/spt-server/.quartermaster/queued")
        );
        assert_eq!(
            dirs.cache_dir(),
            PathBuf::from("/home/user/spt-server/quartermaster-cache")
        );
    }

    #[test]
    fn validate_spt_dir_works_with_spt_server_path() {
        let tmp = tempfile::tempdir().unwrap();
        let dirs = QumaDirs::from_root(tmp.path().to_path_buf());
        std::fs::create_dir_all(dirs.spt_server.join("SPT/SPT_Data/configs")).unwrap();
        std::fs::create_dir_all(dirs.spt_server.join("SPT/user/mods")).unwrap();
        std::fs::create_dir_all(dirs.spt_server.join("BepInEx/plugins")).unwrap();
        std::fs::write(dirs.spt_server.join("SPT/SPT.Server.exe"), "").unwrap();
        std::fs::write(dirs.spt_server.join("SPT/SPT_Data/configs/core.json"), "{}").unwrap();

        assert!(crate::spt::detect::validate_spt_dir(&dirs.spt_server).is_ok());
    }

    #[test]
    fn detect_new_layout() {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path();
        std::fs::write(root.join("quartermaster.toml"), "").unwrap();
        let spt = root.join("spt-server");
        std::fs::create_dir_all(spt.join("SPT/SPT_Data/configs")).unwrap();
        std::fs::create_dir_all(spt.join("SPT/user/mods")).unwrap();
        std::fs::create_dir_all(spt.join("BepInEx/plugins")).unwrap();
        std::fs::write(spt.join("SPT/SPT.Server.exe"), "").unwrap();
        std::fs::write(spt.join("SPT/SPT_Data/configs/core.json"), "{}").unwrap();

        let dirs = QumaDirs::detect(Some(root), None).unwrap();
        assert!(!dirs.is_legacy());
        assert_eq!(dirs.spt_server, spt);
    }

    #[test]
    fn detect_legacy_layout() {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path();
        std::fs::create_dir_all(root.join("SPT/SPT_Data/configs")).unwrap();
        std::fs::create_dir_all(root.join("SPT/user/mods")).unwrap();
        std::fs::create_dir_all(root.join("BepInEx/plugins")).unwrap();
        std::fs::write(root.join("SPT/SPT.Server.exe"), "").unwrap();
        std::fs::write(root.join("SPT/SPT_Data/configs/core.json"), "{}").unwrap();

        let dirs = QumaDirs::detect(Some(root), None).unwrap();
        assert!(dirs.is_legacy());
        assert_eq!(dirs.spt_server, root.to_path_buf());
    }

    #[test]
    fn detect_env_var_quma_dir() {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path();
        std::fs::write(root.join("quartermaster.toml"), "").unwrap();
        let spt = root.join("spt-server");
        std::fs::create_dir_all(spt.join("SPT/SPT_Data/configs")).unwrap();
        std::fs::create_dir_all(spt.join("SPT/user/mods")).unwrap();
        std::fs::create_dir_all(spt.join("BepInEx/plugins")).unwrap();
        std::fs::write(spt.join("SPT/SPT.Server.exe"), "").unwrap();
        std::fs::write(spt.join("SPT/SPT_Data/configs/core.json"), "{}").unwrap();

        temp_env::with_vars(
            [
                ("QUMA_DIR", Some(root.to_str().unwrap())),
                ("QUMA_SPT_DIR", None),
            ],
            || {
                let dirs = QumaDirs::detect(None, None).unwrap();
                assert!(!dirs.is_legacy());
                assert_eq!(dirs.root, root.to_path_buf());
            },
        );
    }

    #[test]
    fn detect_deprecated_env_var() {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path();
        std::fs::create_dir_all(root.join("SPT/SPT_Data/configs")).unwrap();
        std::fs::create_dir_all(root.join("SPT/user/mods")).unwrap();
        std::fs::create_dir_all(root.join("BepInEx/plugins")).unwrap();
        std::fs::write(root.join("SPT/SPT.Server.exe"), "").unwrap();
        std::fs::write(root.join("SPT/SPT_Data/configs/core.json"), "{}").unwrap();

        temp_env::with_vars(
            [
                ("QUMA_SPT_DIR", Some(root.to_str().unwrap())),
                ("QUMA_DIR", None),
            ],
            || {
                let dirs = QumaDirs::detect(None, None).unwrap();
                assert!(dirs.is_legacy());
            },
        );
    }
}
