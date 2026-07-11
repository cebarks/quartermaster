use std::path::PathBuf;

// ponytail: unused until Task 2+ consume it
#[allow(dead_code)]
#[derive(Debug, Clone)]
pub struct QumaDirs {
    pub root: PathBuf,
    pub spt_server: PathBuf,
    pub headless: PathBuf,
    pub overlay: PathBuf,
    legacy: bool,
}

#[allow(dead_code)]
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

    pub fn cache_dir(&self) -> PathBuf {
        if self.legacy {
            self.root.join("quartermaster-cache")
        } else {
            self.root.join("cache")
        }
    }

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

    pub fn core_json(&self) -> PathBuf {
        self.spt_server.join("SPT/SPT_Data/configs/core.json")
    }

    pub fn http_json(&self) -> PathBuf {
        self.spt_server.join("SPT/SPT_Data/configs/http.json")
    }

    pub fn fika_config(&self) -> PathBuf {
        self.spt_server
            .join("SPT/user/mods/fika-server/assets/configs/fika.jsonc")
    }

    // -- Headless paths --

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
}
