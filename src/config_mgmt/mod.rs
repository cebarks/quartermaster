pub mod git;

use std::path::{Path, PathBuf};

use anyhow::{Context, Result};

use crate::db::Database;

pub use git::{ConfigHistoryRepo, HistoryEntry};

pub struct ConfigManager {
    pub history: ConfigHistoryRepo,
    spt_dir: PathBuf,
}

pub struct ModConfigSet {
    #[allow(dead_code)] // ponytail: used in later tasks
    pub mod_id: i64,
    #[allow(dead_code)] // ponytail: used in later tasks
    pub mod_name: String,
    #[allow(dead_code)] // ponytail: used in later tasks
    pub config_files: Vec<ConfigFile>,
}

pub struct ConfigFile {
    #[allow(dead_code)] // ponytail: used in later tasks
    pub rel_path: PathBuf,
    #[allow(dead_code)] // ponytail: used in later tasks
    pub filename: String,
    #[allow(dead_code)] // ponytail: used in later tasks
    pub size: u64,
}

impl ConfigManager {
    pub fn new(spt_dir: &Path) -> Self {
        let history_path = spt_dir.join("quartermaster/config-history");
        Self {
            history: ConfigHistoryRepo::new(history_path),
            spt_dir: spt_dir.to_path_buf(),
        }
    }

    /// Discover all JSON/JSONC config files for installed server mods.
    /// Combines DB records with filesystem scan for lazily-created configs.
    pub fn discover_configs(&self, db: &Database) -> Result<Vec<ModConfigSet>> {
        let mods = db.list_mods()?;
        let mut result = Vec::new();

        for m in &mods {
            let mod_dir_name = self.find_mod_dir(&m.name)?;
            let Some(mod_dir_name) = mod_dir_name else {
                continue;
            };
            let config_dir = self
                .spt_dir
                .join("SPT/user/mods")
                .join(&mod_dir_name)
                .join("config");
            if !config_dir.is_dir() {
                continue;
            }

            let mut config_files = Vec::new();
            Self::scan_config_dir(&config_dir, &config_dir, &mut config_files)?;

            if !config_files.is_empty() {
                result.push(ModConfigSet {
                    mod_id: m.id,
                    mod_name: m.name.clone(),
                    config_files,
                });
            }
        }

        Ok(result)
    }

    pub fn scan_config_dir(base: &Path, dir: &Path, out: &mut Vec<ConfigFile>) -> Result<()> {
        let entries =
            std::fs::read_dir(dir).with_context(|| format!("failed to read {}", dir.display()))?;
        for entry in entries {
            let entry = entry?;
            let path = entry.path();
            if path.is_dir() {
                Self::scan_config_dir(base, &path, out)?;
            } else if let Some(ext) = path.extension().and_then(|e| e.to_str()) {
                if ext.eq_ignore_ascii_case("json") || ext.eq_ignore_ascii_case("jsonc") {
                    let metadata = std::fs::metadata(&path)?;
                    let filename = path
                        .file_name()
                        .and_then(|n| n.to_str())
                        .unwrap_or("unknown")
                        .to_string();
                    let rel_path = path.strip_prefix(base).unwrap_or(&path).to_path_buf();
                    out.push(ConfigFile {
                        rel_path,
                        filename,
                        size: metadata.len(),
                    });
                }
            }
        }
        Ok(())
    }

    /// Find the actual directory name for a mod under SPT/user/mods/.
    /// Mod names in the DB may not match the directory name exactly.
    pub fn find_mod_dir(&self, mod_name: &str) -> Result<Option<String>> {
        let mods_dir = self.spt_dir.join("SPT/user/mods");
        if !mods_dir.is_dir() {
            return Ok(None);
        }
        // Check installed_files to find the actual directory
        // Fallback: scan directories for case-insensitive match
        let entries = std::fs::read_dir(&mods_dir)?;
        let lower_name = mod_name.to_lowercase();
        for entry in entries {
            let entry = entry?;
            if entry.file_type()?.is_dir() {
                let dir_name = entry.file_name();
                let dir_str = dir_name.to_string_lossy();
                if dir_str.to_lowercase() == lower_name
                    || dir_str.to_lowercase().replace(['-', '_', ' '], "")
                        == lower_name.replace(['-', '_', ' '], "")
                {
                    return Ok(Some(dir_str.into_owned()));
                }
            }
        }
        Ok(None)
    }

    /// Read a config file, stripping UTF-8 BOM if present.
    #[allow(dead_code)] // ponytail: used in later tasks
    pub fn read_config(&self, mod_dir: &str, config_rel_path: &Path) -> Result<String> {
        let path = self.config_path(mod_dir, config_rel_path);
        let content = std::fs::read_to_string(&path)
            .with_context(|| format!("failed to read {}", path.display()))?;
        Ok(strip_bom(&content).to_string())
    }

    /// Validate and save a config file. Returns Ok(true) if changed, Ok(false) if no change.
    #[allow(dead_code)] // ponytail: used in later tasks
    pub fn save_config(
        &self,
        mod_dir: &str,
        config_rel_path: &Path,
        content: &str,
        author: &str,
    ) -> Result<bool> {
        // Validate JSONC
        jsonc_parser::parse_to_serde_value(content, &Default::default())
            .map_err(|e| anyhow::anyhow!("Invalid JSON/JSONC: {e}"))?;

        let file_path = self.config_path(mod_dir, config_rel_path);
        let current = std::fs::read_to_string(&file_path).unwrap_or_default();
        let current = strip_bom(&current);

        if current == content {
            return Ok(false);
        }

        // Build the history repo path for this file
        let history_rel = Path::new("user/mods")
            .join(mod_dir)
            .join("config")
            .join(config_rel_path);

        // Snapshot the "before" state if this file hasn't been tracked yet
        if !self.history.has_file(&history_rel) && !current.is_empty() {
            self.history.snapshot(
                &history_rel,
                current,
                "quartermaster",
                &format!("Initial snapshot of {}", config_rel_path.display()),
            )?;
        }

        // Write to disk atomically
        let dir = file_path
            .parent()
            .context("config file has no parent dir")?;
        let tmp = tempfile::NamedTempFile::new_in(dir)?;
        std::fs::write(tmp.path(), content)?;
        tmp.persist(&file_path)
            .with_context(|| format!("failed to persist {}", file_path.display()))?;

        // Commit new version to history
        self.history.snapshot(
            &history_rel,
            content,
            author,
            &format!("Update {}/{}", mod_dir, config_rel_path.display()),
        )?;

        Ok(true)
    }

    /// Restore a config file to a previous revision.
    #[allow(dead_code)] // ponytail: used in later tasks
    pub fn restore_config(
        &self,
        mod_dir: &str,
        config_rel_path: &Path,
        rev: &str,
        author: &str,
    ) -> Result<()> {
        let history_rel = Path::new("user/mods")
            .join(mod_dir)
            .join("config")
            .join(config_rel_path);

        let old_content = self.history.content_at_rev(&history_rel, rev)?;

        // Write restored content to disk
        let file_path = self.config_path(mod_dir, config_rel_path);
        let dir = file_path
            .parent()
            .context("config file has no parent dir")?;
        let tmp = tempfile::NamedTempFile::new_in(dir)?;
        std::fs::write(tmp.path(), &old_content)?;
        tmp.persist(&file_path)
            .with_context(|| format!("failed to persist {}", file_path.display()))?;

        // Commit as a new version (restore is a forward commit, not a reset)
        self.history.snapshot(
            &history_rel,
            &old_content,
            author,
            &format!(
                "Restore {}/{} to revision {}",
                mod_dir,
                config_rel_path.display(),
                &rev[..7.min(rev.len())]
            ),
        )?;

        Ok(())
    }

    /// Get history for a config file.
    #[allow(dead_code)] // ponytail: used in later tasks
    pub fn history(&self, mod_dir: &str, config_rel_path: &Path) -> Result<Vec<HistoryEntry>> {
        let history_rel = Path::new("user/mods")
            .join(mod_dir)
            .join("config")
            .join(config_rel_path);
        self.history.history(&history_rel)
    }

    /// Get content at a specific revision.
    #[allow(dead_code)] // ponytail: used in later tasks
    pub fn content_at_rev(
        &self,
        mod_dir: &str,
        config_rel_path: &Path,
        rev: &str,
    ) -> Result<String> {
        let history_rel = Path::new("user/mods")
            .join(mod_dir)
            .join("config")
            .join(config_rel_path);
        self.history.content_at_rev(&history_rel, rev)
    }

    #[allow(dead_code)] // ponytail: used in later tasks
    fn config_path(&self, mod_dir: &str, config_rel_path: &Path) -> PathBuf {
        self.spt_dir
            .join("SPT/user/mods")
            .join(mod_dir)
            .join("config")
            .join(config_rel_path)
    }
}

#[allow(dead_code)] // ponytail: used in later tasks
fn strip_bom(s: &str) -> &str {
    s.strip_prefix('\u{feff}').unwrap_or(s)
}
