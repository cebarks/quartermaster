pub mod config;
pub mod metadata;

use std::path::{Path, PathBuf};

use anyhow::{bail, Context, Result};
use serde_json::Value;

pub use config::SvmConfig;

pub const SVM_FORGE_ID: i64 = 236;
const LOADER_FILE: &str = "Loader/loader.json";
const PRESETS_DIR: &str = "Presets";

pub struct SvmManager {
    svm_dir: PathBuf,
    dirs: Option<crate::dirs::QumaDirs>,
    relative: PathBuf,
    active_preset: String,
    config: SvmConfig,
    raw_json: Value,
    unknown_fields: Vec<String>,
    available_presets: Vec<String>,
    dirty: bool,
}

impl SvmManager {
    pub fn detect_dirs(dirs: &crate::dirs::QumaDirs) -> Option<Self> {
        if dirs.is_legacy() {
            return Self::detect(&dirs.spt_server);
        }
        let mounted = dirs.spt_overlay().is_mounted().ok()?;
        let roots = if mounted {
            vec![dirs.spt_runtime()]
        } else {
            vec![
                dirs.runtime_upper(),
                dirs.mod_overlay(),
                dirs.spt_server.clone(),
            ]
        };
        let mut candidates = std::collections::BTreeSet::new();
        let runtime = crate::spt::layout::detect_layout(&dirs.spt_server)
            .ok()?
            .directory();
        for root in &roots {
            let mods = Path::new(runtime).join("user/mods");
            if let Ok(entries) = std::fs::read_dir(root.join(&mods)) {
                for entry in entries.flatten() {
                    if entry
                        .file_name()
                        .to_string_lossy()
                        .to_lowercase()
                        .contains("svm")
                    {
                        candidates.insert(mods.join(entry.file_name()));
                    }
                }
            }
        }
        for relative in candidates {
            // Runtime upper can retain user-edited loader/preset files after the
            // package is removed. Those files alone are not an installation.
            let payload = relative.join("ServerValueModifier.dll");
            let installed = [dirs.mod_overlay(), dirs.spt_server.clone()]
                .iter()
                .filter_map(|root| crate::spt::scoped::ScopedDir::open(root).ok())
                .any(|root| root.read_overlay_file(&payload).ok().flatten().is_some());
            if !installed {
                continue;
            }
            let visible = if mounted {
                crate::spt::scoped::ScopedDir::open(&dirs.spt_runtime())
                    .and_then(|scope| scope.read_file(&payload))
                    .is_ok()
            } else {
                crate::spt::layered::LayeredFiles::open(dirs)
                    .and_then(|layers| layers.read(&payload, false))
                    .is_ok()
            };
            if !visible {
                continue;
            }
            let svm_dir = if mounted {
                dirs.spt_runtime().join(&relative)
            } else {
                dirs.spt_server.join(&relative)
            };
            if let Some(manager) = Self::initialize(svm_dir, Some(dirs.clone()), relative) {
                return Some(manager);
            }
        }
        None
    }

    fn read_file(&self, relative: &Path) -> Result<Vec<u8>> {
        use std::io::Read;
        if let Some(dirs) = &self.dirs {
            if !dirs.spt_overlay().is_mounted()? {
                let mut file = crate::spt::layered::LayeredFiles::open(dirs)?
                    .read(&self.relative.join(relative), false)?;
                let mut data = Vec::new();
                file.read_to_end(&mut data)?;
                return Ok(data);
            }
            let scope = crate::spt::scoped::ScopedDir::open(&dirs.spt_runtime())?;
            let mut file = scope.read_file(&self.relative.join(relative))?;
            let mut data = Vec::new();
            file.read_to_end(&mut data)?;
            return Ok(data);
        }
        Ok(std::fs::read(self.svm_dir.join(relative))?)
    }

    fn read_text(&self, relative: &Path) -> Result<String> {
        Ok(String::from_utf8(self.read_file(relative)?)?)
    }

    fn write_scope(
        &self,
    ) -> Result<(
        crate::spt::scoped::ScopedDir,
        Option<std::fs::File>,
        PathBuf,
    )> {
        use std::os::fd::AsRawFd;

        let Some(dirs) = &self.dirs else {
            return Ok((
                crate::spt::scoped::ScopedDir::open(&self.svm_dir)?,
                None,
                PathBuf::new(),
            ));
        };
        if !dirs.spt_overlay().is_mounted()? {
            bail!("SVM editing requires the server runtime overlay to be mounted");
        }
        let mount = std::fs::File::open(dirs.spt_runtime())?;
        let mount_id = |file: &std::fs::File| -> Result<String> {
            let info = std::fs::read_to_string(format!("/proc/self/fdinfo/{}", file.as_raw_fd()))?;
            Ok(info
                .lines()
                .find_map(|line| line.strip_prefix("mnt_id:").map(str::trim))
                .context("missing mount identity")?
                .to_owned())
        };
        let id = mount_id(&mount)?;
        let current = std::fs::File::open(dirs.spt_runtime())?;
        if mount_id(&current)? != id || !dirs.spt_overlay().is_mounted()? {
            bail!("server overlay changed while opening SVM configuration");
        }
        let info = std::fs::read_to_string("/proc/self/mountinfo")?;
        let is_overlay = info.lines().any(|line| {
            line.split_once(" - ").is_some_and(|(before, after)| {
                before.split_whitespace().next() == Some(id.as_str())
                    && after.split_whitespace().next() == Some("fuse.fuse-overlayfs")
            })
        });
        if !is_overlay {
            bail!("SVM configuration is not on the mounted server overlay");
        }
        let path = PathBuf::from(format!("/proc/self/fd/{}", mount.as_raw_fd()));
        let scope = crate::spt::scoped::ScopedDir::open(&path)?;
        Ok((scope, Some(mount), self.relative.clone()))
    }

    fn write_file(&self, relative: &Path, bytes: &[u8], create_new: bool) -> Result<()> {
        use std::io::Write;

        let (scope, _mount, root) = self.write_scope()?;
        let path = root.join(relative);
        if !create_new {
            return scope.replace_file(&path, bytes);
        }
        let mut file = scope.create_new_file(&path)?;
        if let Err(error) = file.write_all(bytes) {
            drop(file);
            let _ = scope.remove_file(&path);
            return Err(error.into());
        }
        Ok(())
    }

    fn remove_file(&self, relative: &Path) -> Result<()> {
        let (scope, _mount, root) = self.write_scope()?;
        scope.remove_file(&root.join(relative))
    }

    fn effective_presets(&self) -> Result<Vec<String>> {
        let Some(dirs) = &self.dirs else {
            return Self::scan_presets(&self.svm_dir.join(PRESETS_DIR));
        };
        if dirs.spt_overlay().is_mounted()? {
            return Self::scan_presets(&dirs.spt_runtime().join(&self.relative).join(PRESETS_DIR));
        }
        let mut names = std::collections::BTreeSet::new();
        for root in [
            dirs.runtime_upper(),
            dirs.mod_overlay(),
            dirs.spt_server.clone(),
        ] {
            let scope = match crate::spt::scoped::ScopedDir::open(&root) {
                Ok(scope) => scope,
                Err(error)
                    if error
                        .downcast_ref::<std::io::Error>()
                        .is_some_and(|e| e.kind() == std::io::ErrorKind::NotFound) =>
                {
                    continue
                }
                Err(error) => return Err(error),
            };
            let (entries, blocks_lower) =
                scope.overlay_directory(&self.relative.join(PRESETS_DIR))?;
            for name in entries {
                if let Some(name) = name.to_str().and_then(|n| n.strip_suffix(".json")) {
                    names.insert(name.to_owned());
                }
            }
            if blocks_lower {
                break;
            }
        }
        Ok(names
            .into_iter()
            .filter(|name| {
                self.read_file(&Path::new(PRESETS_DIR).join(format!("{name}.json")))
                    .is_ok()
            })
            .collect())
    }

    pub fn import_preset(&mut self, name: &str, json: &str) -> Result<()> {
        Self::validate_preset_name(name)?;
        let json = json.strip_prefix('\u{feff}').unwrap_or(json);
        let _: SvmConfig = serde_json::from_str(json).context("Invalid SVM preset")?;
        self.write_file(
            &Path::new(PRESETS_DIR).join(format!("{name}.json")),
            json.as_bytes(),
            true,
        )?;
        self.available_presets.push(name.to_owned());
        self.available_presets.sort();
        Ok(())
    }

    /// Detect SVM installation and initialize manager.
    /// Returns None if SVM is not installed or detection fails.
    pub fn detect(spt_dir: &Path) -> Option<Self> {
        let svm_dir = crate::config::find_svm_dir(spt_dir)?;
        Self::initialize(svm_dir, None, PathBuf::new())
    }

    fn initialize(
        svm_dir: PathBuf,
        dirs: Option<crate::dirs::QumaDirs>,
        relative: PathBuf,
    ) -> Option<Self> {
        let mut mgr = Self {
            svm_dir,
            dirs,
            relative,
            active_preset: String::new(),
            config: SvmConfig::default(),
            raw_json: Value::Null,
            unknown_fields: Vec::new(),
            available_presets: Vec::new(),
            dirty: false,
        };
        let loader_content = mgr.read_text(Path::new(LOADER_FILE)).ok()?;
        let loader_content = loader_content
            .strip_prefix('\u{feff}')
            .unwrap_or(&loader_content);
        let loader_json: Value = serde_json::from_str(loader_content).ok()?;
        let active_preset = loader_json
            .get("CurrentlySelectedPreset")
            .and_then(|v| v.as_str())
            .filter(|s| !s.is_empty() && *s != "null")
            .map(str::to_owned)
            .unwrap_or_default();

        mgr.available_presets = mgr.effective_presets().ok()?;
        mgr.active_preset = active_preset.clone();

        if active_preset.is_empty() {
            return Some(mgr);
        }
        match mgr.load_preset_from_disk(&active_preset) {
            Ok((config, raw_json, unknown_fields)) => {
                mgr.config = config;
                mgr.raw_json = raw_json;
                mgr.unknown_fields = unknown_fields;
                Some(mgr)
            }
            Err(e) => {
                tracing::warn!("Failed to load SVM preset '{}': {}", active_preset, e);
                None
            }
        }
    }

    /// Scan presets directory and return sorted list of preset names.
    fn scan_presets(presets_dir: &Path) -> Result<Vec<String>> {
        let mut presets = Vec::new();
        if !presets_dir.exists() {
            return Ok(presets);
        }

        for entry in std::fs::read_dir(presets_dir)? {
            let entry = entry?;
            let path = entry.path();
            if path.is_file() && path.extension().and_then(|s| s.to_str()) == Some("json") {
                if let Some(name) = path.file_stem().and_then(|s| s.to_str()) {
                    presets.push(name.to_string());
                }
            }
        }
        presets.sort();
        Ok(presets)
    }

    /// Validate a preset name for safety and OS compatibility.
    pub fn validate_preset_name(name: &str) -> Result<()> {
        if name.is_empty() {
            bail!("Preset name cannot be empty");
        }

        if name.len() > 64 {
            bail!("Preset name too long (max 64 characters)");
        }

        // Check for path traversal characters
        if name.contains('/') || name.contains('\\') || name.contains("..") {
            bail!("Preset name contains invalid path characters");
        }

        // Check all characters are safe
        for ch in name.chars() {
            if !ch.is_alphanumeric() && ch != ' ' && ch != '-' && ch != '_' {
                bail!("Preset name contains invalid character: '{}'", ch);
            }
        }

        // Check for OS-reserved names (Windows)
        let upper = name.to_uppercase();
        let reserved = [
            "CON", "PRN", "AUX", "NUL", "COM1", "COM2", "COM3", "COM4", "COM5", "COM6", "COM7",
            "COM8", "COM9", "LPT1", "LPT2", "LPT3", "LPT4", "LPT5", "LPT6", "LPT7", "LPT8", "LPT9",
        ];
        if reserved.contains(&upper.as_str()) {
            bail!("Preset name '{}' is reserved by the operating system", name);
        }

        Ok(())
    }

    /// Load a preset from disk and return config, raw JSON, and unknown fields.
    fn load_preset_from_disk(&self, name: &str) -> Result<(SvmConfig, Value, Vec<String>)> {
        Self::validate_preset_name(name)?;
        let content = self
            .read_text(&Path::new(PRESETS_DIR).join(format!("{name}.json")))
            .with_context(|| format!("Failed to read preset '{}'", name))?;
        // SVM is a .NET mod — presets may have a UTF-8 BOM
        let content = content.strip_prefix('\u{feff}').unwrap_or(&content);

        // Parse as raw JSON
        let raw_json: Value = serde_json::from_str(content)
            .with_context(|| format!("Failed to parse preset '{}' as JSON", name))?;

        // Parse as typed config
        let config: SvmConfig = serde_json::from_str(content)
            .with_context(|| format!("Failed to deserialize preset '{}'", name))?;

        // Serialize typed config back to JSON to compare keys
        let typed_json = serde_json::to_value(&config)?;

        // Find unknown fields
        let unknown_fields = Self::collect_unknown_keys(&raw_json, &typed_json, "");

        Ok((config, raw_json, unknown_fields))
    }

    /// Recursively collect keys present in raw but not in typed.
    fn collect_unknown_keys(raw: &Value, typed: &Value, prefix: &str) -> Vec<String> {
        let mut unknown = Vec::new();

        if let (Some(raw_obj), Some(typed_obj)) = (raw.as_object(), typed.as_object()) {
            for (key, raw_value) in raw_obj {
                let path = if prefix.is_empty() {
                    key.clone()
                } else {
                    format!("{}.{}", prefix, key)
                };

                if let Some(typed_value) = typed_obj.get(key) {
                    // Key exists in both, recurse if both are objects
                    if raw_value.is_object() && typed_value.is_object() {
                        unknown.extend(Self::collect_unknown_keys(raw_value, typed_value, &path));
                    }
                } else {
                    // Key exists in raw but not typed
                    unknown.push(path);
                }
            }
        }

        unknown
    }

    /// Deep merge: overlay values into base, preserving unknown keys in base.
    fn deep_merge(base: &mut Value, overlay: &Value) {
        match (base, overlay) {
            (Value::Object(base_obj), Value::Object(overlay_obj)) => {
                for (key, overlay_value) in overlay_obj {
                    if let Some(base_value) = base_obj.get_mut(key) {
                        // Recursively merge if both are objects
                        if base_value.is_object() && overlay_value.is_object() {
                            Self::deep_merge(base_value, overlay_value);
                        } else {
                            // Overwrite with overlay value
                            *base_value = overlay_value.clone();
                        }
                    } else {
                        // Key doesn't exist in base, add it
                        base_obj.insert(key.clone(), overlay_value.clone());
                    }
                }
            }
            _ => {
                // Not both objects, can't merge
            }
        }
    }

    /// Get the current config.
    pub fn config(&self) -> &SvmConfig {
        &self.config
    }

    /// Check if config has been modified since last save/load.
    pub fn is_dirty(&self) -> bool {
        self.dirty
    }

    /// Clear the dirty flag.
    pub fn clear_dirty(&mut self) {
        self.dirty = false;
    }

    pub(crate) fn mark_dirty(&mut self) {
        self.dirty = true;
    }

    /// Get the name of the currently active preset.
    pub fn active_preset_name(&self) -> &str {
        &self.active_preset
    }

    /// Get the list of available presets.
    pub fn list_presets(&self) -> &[String] {
        &self.available_presets
    }

    /// Load a preset and make it active.
    #[allow(dead_code)]
    pub fn load_preset(&mut self, name: &str) -> Result<()> {
        self.set_active_preset(name)
    }

    /// Save a config to a preset file.
    pub fn save_preset(&mut self, name: &str, config: &SvmConfig) -> Result<()> {
        if name.is_empty() || name != self.active_preset {
            bail!("Select an active preset before editing");
        }
        // Serialize config to JSON
        let config_json = serde_json::to_value(config)?;

        // Deep merge into raw_json to preserve unknown fields
        let mut merged = self.raw_json.clone();
        Self::deep_merge(&mut merged, &config_json);

        // Write to file
        let json_str = serde_json::to_string_pretty(&merged)?;
        self.write_file(
            &Path::new(PRESETS_DIR).join(format!("{name}.json")),
            json_str.as_bytes(),
            false,
        )
        .with_context(|| format!("Failed to write preset '{}'", name))?;

        // Update internal state
        self.config = config.clone();
        self.raw_json = merged.clone();
        self.unknown_fields = Self::collect_unknown_keys(&merged, &config_json, "");
        self.dirty = true;

        Ok(())
    }

    /// Set the active preset and load it.
    pub fn set_active_preset(&mut self, name: &str) -> Result<()> {
        Self::validate_preset_name(name)?;
        // Verify preset exists
        if !self.available_presets.contains(&name.to_string()) {
            bail!("Preset '{}' does not exist", name);
        }

        // Validate and load the target before changing the loader.
        let (config, raw_json, unknown_fields) = self.load_preset_from_disk(name)?;
        let loader_content = self.read_text(Path::new(LOADER_FILE))?;
        let loader_content = loader_content
            .strip_prefix('\u{feff}')
            .unwrap_or(&loader_content);
        let mut loader_json: Value = serde_json::from_str(loader_content)?;
        loader_json
            .as_object_mut()
            .context("SVM loader must be an object")?
            .insert("CurrentlySelectedPreset".into(), name.into());
        self.write_file(
            Path::new(LOADER_FILE),
            serde_json::to_string_pretty(&loader_json)?.as_bytes(),
            false,
        )?;

        self.dirty |= self.active_preset != name;
        self.active_preset = name.to_string();
        self.config = config;
        self.raw_json = raw_json;
        self.unknown_fields = unknown_fields;

        Ok(())
    }

    /// Create a new preset with default config.
    pub fn create_preset(&mut self, name: &str) -> Result<()> {
        Self::validate_preset_name(name)?;

        // Check if preset already exists
        if self.available_presets.contains(&name.to_string()) {
            bail!("Preset '{}' already exists", name);
        }

        // Write default config to file
        let default_config = SvmConfig::default();
        let json_str = serde_json::to_string_pretty(&default_config)?;
        self.write_file(
            &Path::new(PRESETS_DIR).join(format!("{name}.json")),
            json_str.as_bytes(),
            true,
        )
        .with_context(|| format!("Failed to create preset '{}'", name))?;

        // Add to available presets
        self.available_presets.push(name.to_string());
        self.available_presets.sort();

        Ok(())
    }

    /// Duplicate a preset.
    pub fn duplicate_preset(&mut self, source: &str, dest: &str) -> Result<()> {
        Self::validate_preset_name(source)?;
        Self::validate_preset_name(dest)?;

        // Check source exists
        if !self.available_presets.contains(&source.to_string()) {
            bail!("Source preset '{}' does not exist", source);
        }

        // Check destination doesn't exist
        if self.available_presets.contains(&dest.to_string()) {
            bail!("Preset '{}' already exists", dest);
        }

        let (scope, _mount, root) = self.write_scope()?;
        scope
            .duplicate_file(
                &root.join(PRESETS_DIR).join(format!("{source}.json")),
                &root.join(PRESETS_DIR).join(format!("{dest}.json")),
            )
            .with_context(|| format!("Failed to duplicate preset '{}' to '{}'", source, dest))?;

        // Add to available presets
        self.available_presets.push(dest.to_string());
        self.available_presets.sort();

        Ok(())
    }

    /// Delete a preset.
    pub fn delete_preset(&mut self, name: &str) -> Result<()> {
        Self::validate_preset_name(name)?;
        // Cannot delete active preset
        if name == self.active_preset {
            bail!("Cannot delete the currently active preset '{}'", name);
        }

        // Remove file
        self.remove_file(&Path::new(PRESETS_DIR).join(format!("{name}.json")))
            .with_context(|| format!("Failed to delete preset '{}'", name))?;

        // Remove from available presets
        self.available_presets.retain(|p| p != name);

        Ok(())
    }

    /// Reload from disk (discard any unsaved changes).
    pub fn reload_from_disk(&mut self) -> Result<()> {
        // Re-read loader.json
        let loader_content = self.read_text(Path::new(LOADER_FILE))?;
        let loader_content = loader_content
            .strip_prefix('\u{feff}')
            .unwrap_or(&loader_content);
        let loader_json: Value = serde_json::from_str(loader_content)?;
        let active_preset = loader_json
            .get("CurrentlySelectedPreset")
            .and_then(|v| v.as_str())
            .filter(|s| !s.is_empty() && *s != "null")
            .unwrap_or_default()
            .to_string();

        // Re-scan presets
        self.available_presets = self.effective_presets()?;

        // Re-load active preset
        let (config, raw_json, unknown_fields) = if active_preset.is_empty() {
            (SvmConfig::default(), Value::Null, Vec::new())
        } else {
            self.load_preset_from_disk(&active_preset)?
        };
        self.active_preset = active_preset;
        self.config = config;
        self.raw_json = raw_json;
        self.unknown_fields = unknown_fields;
        Ok(())
    }

    /// Get the list of unknown field paths.
    pub fn unknown_fields(&self) -> &[String] {
        &self.unknown_fields
    }

    pub fn read_preset(&self, name: &str) -> Result<String> {
        Self::validate_preset_name(name)?;
        self.read_text(&Path::new(PRESETS_DIR).join(format!("{name}.json")))
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;
    use std::fs;
    use tempfile::TempDir;

    fn setup_svm_dir(tmp: &TempDir) -> PathBuf {
        let spt_dir = tmp.path().to_path_buf();
        let svm_dir = spt_dir.join("SPT/user/mods/[SVM] Server Value Modifier");
        fs::create_dir_all(svm_dir.join("Loader")).unwrap();
        fs::create_dir_all(svm_dir.join("Presets")).unwrap();
        let loader = serde_json::json!({"CurrentlySelectedPreset": "Default"});
        fs::write(svm_dir.join("Loader/loader.json"), loader.to_string()).unwrap();
        let config = SvmConfig::default();
        fs::write(
            svm_dir.join("Presets/Default.json"),
            serde_json::to_string_pretty(&config).unwrap(),
        )
        .unwrap();
        spt_dir
    }

    #[test]
    fn detect_finds_svm_installation() {
        let tmp = TempDir::new().unwrap();
        let spt_dir = setup_svm_dir(&tmp);
        let mgr = SvmManager::detect(&spt_dir).expect("should detect SVM");
        assert_eq!(mgr.active_preset_name(), "Default");
        assert_eq!(mgr.list_presets(), &["Default"]);
        assert!(!mgr.is_dirty());
    }

    #[test]
    fn detect_returns_none_when_no_svm() {
        let tmp = TempDir::new().unwrap();
        assert!(SvmManager::detect(tmp.path()).is_none());
    }

    #[test]
    fn duplicate_preserves_private_mode() {
        use std::os::unix::fs::{MetadataExt, PermissionsExt};
        let tmp = TempDir::new().unwrap();
        let root = setup_svm_dir(&tmp);
        let presets = root.join("SPT/user/mods/[SVM] Server Value Modifier/Presets");
        fs::set_permissions(
            presets.join("Default.json"),
            fs::Permissions::from_mode(0o600),
        )
        .unwrap();
        let mut manager = SvmManager::detect(&root).unwrap();
        manager.duplicate_preset("Default", "Copy").unwrap();
        assert_eq!(
            fs::metadata(presets.join("Copy.json")).unwrap().mode() & 0o777,
            0o600
        );
    }

    #[test]
    #[ignore = "requires setfacl and getfacl on an ACL-enabled filesystem"]
    fn duplicate_preserves_access_acl_without_inheriting_parent_acl() {
        use std::process::Command;
        let tmp = TempDir::new().unwrap();
        let root = setup_svm_dir(&tmp);
        let presets = root.join("SPT/user/mods/[SVM] Server Value Modifier/Presets");
        let source = presets.join("Default.json");
        assert!(Command::new("setfacl")
            .args(["-m", "u:12345:r--"])
            .arg(&source)
            .status()
            .unwrap()
            .success());
        assert!(Command::new("setfacl")
            .args(["-m", "d:u:12346:rwx"])
            .arg(&presets)
            .status()
            .unwrap()
            .success());
        let mut manager = SvmManager::detect(&root).unwrap();
        manager.duplicate_preset("Default", "Copy").unwrap();
        let acl = |path: &Path| {
            Command::new("getfacl")
                .args(["-cp"])
                .arg(path)
                .output()
                .unwrap()
                .stdout
        };
        assert_eq!(acl(&source), acl(&presets.join("Copy.json")));
    }

    #[test]
    fn offline_hidden_unreadable_presets_are_not_enumerated() {
        use std::os::unix::fs::PermissionsExt;
        let tmp = TempDir::new().unwrap();
        let dirs = crate::dirs::QumaDirs::from_root(tmp.path().to_path_buf());
        let rel = Path::new("SPT_Runtime/user/mods/SVM");
        let lower = dirs.mod_overlay().join(rel);
        let upper = dirs.runtime_upper().join(rel);
        fs::create_dir_all(lower.join("Loader")).unwrap();
        fs::create_dir_all(lower.join("Presets")).unwrap();
        fs::create_dir_all(&upper).unwrap();
        fs::write(lower.join("ServerValueModifier.dll"), b"payload").unwrap();
        fs::write(
            lower.join(LOADER_FILE),
            r#"{"CurrentlySelectedPreset":"null"}"#,
        )
        .unwrap();
        fs::set_permissions(lower.join("Presets"), fs::Permissions::from_mode(0)).unwrap();
        fs::write(upper.join(".wh.Presets"), b"").unwrap();
        assert!(SvmManager::detect_dirs(&dirs)
            .expect("hidden lower directory")
            .list_presets()
            .is_empty());
        fs::remove_file(upper.join(".wh.Presets")).unwrap();
        fs::create_dir(upper.join("Presets")).unwrap();
        fs::write(upper.join("Presets/.wh..wh..opq"), b"").unwrap();
        assert!(SvmManager::detect_dirs(&dirs)
            .expect("opaque upper directory")
            .list_presets()
            .is_empty());
        fs::set_permissions(lower.join("Presets"), fs::Permissions::from_mode(0o700)).unwrap();
    }

    #[test]
    fn detects_inactive_svm_in_mod_overlay_without_writes() {
        let tmp = TempDir::new().unwrap();
        let dirs = crate::dirs::QumaDirs::from_root(tmp.path().to_path_buf());
        let svm_dir = dirs.mod_overlay().join("SPT_Runtime/user/mods/SVM");
        fs::create_dir_all(svm_dir.join("Loader")).unwrap();
        fs::write(svm_dir.join("ServerValueModifier.dll"), b"payload").unwrap();
        let loader = r#"{"CurrentlySelectedPreset":"null","Other":123}"#;
        fs::write(svm_dir.join("Loader/loader.json"), loader).unwrap();
        let mgr = SvmManager::detect_dirs(&dirs).expect("SVM installed in overlay");
        assert_eq!(mgr.active_preset_name(), "");
        assert_eq!(
            fs::read_to_string(svm_dir.join("Loader/loader.json")).unwrap(),
            loader
        );
        assert!(!svm_dir.join("Presets").exists());
    }

    #[test]
    fn detection_ignores_preserved_runtime_settings_after_payload_removed() {
        let tmp = TempDir::new().unwrap();
        let dirs = crate::dirs::QumaDirs::from_root(tmp.path().to_path_buf());
        let relative = Path::new("SPT_Runtime/user/mods/SVM");
        let package = dirs.mod_overlay().join(relative);
        let upper = dirs.runtime_upper().join(relative);
        fs::create_dir_all(&package).unwrap();
        fs::create_dir_all(upper.join("Loader")).unwrap();
        fs::write(package.join("ServerValueModifier.dll"), b"payload").unwrap();
        let loader = r#"{"CurrentlySelectedPreset":"null"}"#;
        fs::write(upper.join(LOADER_FILE), loader).unwrap();
        assert!(SvmManager::detect_dirs(&dirs).is_some());
        fs::remove_file(package.join("ServerValueModifier.dll")).unwrap();
        assert!(SvmManager::detect_dirs(&dirs).is_none());
        assert_eq!(fs::read_to_string(upper.join(LOADER_FILE)).unwrap(), loader);
    }

    #[test]
    fn offline_preset_scan_handles_whiteout_and_hidden_nondirectory() {
        use std::os::fd::AsRawFd;

        let tmp = TempDir::new().unwrap();
        let dirs = crate::dirs::QumaDirs::from_root(tmp.path().to_path_buf());
        let rel = Path::new("SPT_Runtime/user/mods/SVM");
        let package = dirs.mod_overlay().join(rel);
        let upper = dirs.runtime_upper().join(rel);
        let base = dirs.spt_server.join(rel);
        fs::create_dir_all(package.join("Loader")).unwrap();
        fs::create_dir_all(package.join("Presets")).unwrap();
        fs::create_dir_all(&upper).unwrap();
        fs::write(package.join("ServerValueModifier.dll"), b"payload").unwrap();
        fs::write(
            package.join(LOADER_FILE),
            r#"{"CurrentlySelectedPreset":"null"}"#,
        )
        .unwrap();
        fs::write(package.join("Presets/Lower.json"), b"{}").unwrap();
        let whiteout = fs::File::create(upper.join("Presets")).unwrap();
        // SAFETY: the descriptor, attribute name, and value are valid.
        assert_eq!(
            unsafe {
                libc::fsetxattr(
                    whiteout.as_raw_fd(),
                    c"user.fuseoverlayfs.whiteout".as_ptr(),
                    b"y".as_ptr().cast(),
                    1,
                    0,
                )
            },
            0
        );
        assert!(SvmManager::detect_dirs(&dirs)
            .unwrap()
            .list_presets()
            .is_empty());

        fs::remove_file(upper.join("Presets")).unwrap();
        fs::create_dir_all(upper.join("Presets")).unwrap();
        fs::write(upper.join("Presets/Upper.json"), b"{}").unwrap();
        fs::create_dir_all(&base).unwrap();
        fs::write(base.join("Presets"), b"hidden file").unwrap();
        assert_eq!(
            SvmManager::detect_dirs(&dirs).unwrap().list_presets(),
            &["Lower", "Upper"]
        );
    }

    #[test]
    #[ignore = "requires a usable FUSE overlay mount"]
    fn mounted_overlay_writes_presets_through_union_only() {
        let tmp = TempDir::new().unwrap();
        let dirs = crate::dirs::QumaDirs::from_root(tmp.path().to_path_buf());
        let mod_dir = dirs.mod_overlay().join("SPT_Runtime/user/mods/SVM");
        fs::create_dir_all(mod_dir.join("Loader")).unwrap();
        fs::write(mod_dir.join("ServerValueModifier.dll"), b"payload").unwrap();
        let loader = r#"{"CurrentlySelectedPreset":"null","Other":123}"#;
        fs::write(mod_dir.join("Loader/loader.json"), loader).unwrap();
        fs::create_dir_all(&dirs.spt_server).unwrap();
        dirs.spt_overlay().mount().unwrap();
        struct Unmount(crate::overlay::OverlayMount);
        impl Drop for Unmount {
            fn drop(&mut self) {
                self.0.unmount().unwrap();
            }
        }
        let mount = Unmount(dirs.spt_overlay());
        let mut mgr = SvmManager::detect_dirs(&dirs).unwrap();
        mgr.create_preset("Custom").unwrap();
        mgr.duplicate_preset("Custom", "Copy").unwrap();
        assert_eq!(mgr.active_preset_name(), "");
        assert_eq!(
            fs::read_to_string(mod_dir.join("Loader/loader.json")).unwrap(),
            loader
        );
        assert!(!mod_dir.join("Presets/Custom.json").exists());
        assert!(dirs
            .spt_runtime()
            .join("SPT_Runtime/user/mods/SVM/Presets/Custom.json")
            .is_file());
        mgr.set_active_preset("Custom").unwrap();
        assert_eq!(
            fs::read_to_string(mod_dir.join("Loader/loader.json")).unwrap(),
            loader
        );
        assert_eq!(mgr.active_preset_name(), "Custom");
        drop(mount);
        assert!(mgr.create_preset("Offline").is_err());
    }

    #[test]
    fn split_overlay_reads_effective_loader_and_presets_and_rejects_writes() {
        let tmp = TempDir::new().unwrap();
        let dirs = crate::dirs::QumaDirs::from_root(tmp.path().to_path_buf());
        let rel = Path::new("SPT_Runtime/user/mods/SVM");
        let base = dirs.spt_server.join(rel);
        let upper = dirs.runtime_upper().join(rel);
        let overlay = dirs.mod_overlay().join(rel);
        fs::create_dir_all(base.join("Presets")).unwrap();
        fs::create_dir_all(upper.join("Loader")).unwrap();
        fs::create_dir_all(overlay.join("Presets")).unwrap();
        fs::write(overlay.join("ServerValueModifier.dll"), b"payload").unwrap();
        let loader = r#"{"CurrentlySelectedPreset":"Lower","Other":123}"#;
        fs::write(upper.join(LOADER_FILE), loader).unwrap();
        let mut config = SvmConfig::default();
        config.raids.raid_time = 42;
        fs::write(
            base.join("Presets/Lower.json"),
            serde_json::to_vec(&config).unwrap(),
        )
        .unwrap();
        fs::write(
            overlay.join("Presets/Other.json"),
            serde_json::to_vec(&config).unwrap(),
        )
        .unwrap();
        let mut mgr = SvmManager::detect_dirs(&dirs).unwrap();
        assert_eq!(mgr.active_preset_name(), "Lower");
        assert_eq!(mgr.list_presets(), &["Lower", "Other"]);
        assert_eq!(mgr.config().raids.raid_time, 42);
        assert!(mgr
            .set_active_preset("Other")
            .unwrap_err()
            .to_string()
            .contains("runtime overlay to be mounted"));
        assert!(mgr.create_preset("New").is_err());
        assert_eq!(fs::read_to_string(upper.join(LOADER_FILE)).unwrap(), loader);
        mgr.reload_from_disk().unwrap();
        assert_eq!(mgr.active_preset_name(), "Lower");
    }

    #[test]
    fn import_preset_preserves_input_and_does_not_activate() {
        let tmp = TempDir::new().unwrap();
        let spt_dir = setup_svm_dir(&tmp);
        let svm_dir = spt_dir.join("SPT/user/mods/[SVM] Server Value Modifier");
        let loader_path = svm_dir.join("Loader/loader.json");
        let loader = r#"{"CurrentlySelectedPreset":"null","Other":123}"#;
        fs::write(&loader_path, loader).unwrap();
        let mut mgr = SvmManager::detect(&spt_dir).unwrap();
        let mut json = serde_json::to_value(SvmConfig::default()).unwrap();
        json["FutureField"] = serde_json::json!({"retain": true});
        mgr.import_preset("Imported", &json.to_string()).unwrap();
        assert_eq!(fs::read_to_string(loader_path).unwrap(), loader);
        assert_eq!(mgr.active_preset_name(), "");
        assert!(mgr.list_presets().contains(&"Imported".to_string()));
        let imported: Value = serde_json::from_str(
            &fs::read_to_string(svm_dir.join("Presets/Imported.json")).unwrap(),
        )
        .unwrap();
        assert_eq!(imported["FutureField"]["retain"], true);
        assert!(mgr.import_preset("Imported", &json.to_string()).is_err());
    }

    #[test]
    fn save_and_reload_preserves_changes() {
        let tmp = TempDir::new().unwrap();
        let spt_dir = setup_svm_dir(&tmp);
        let mut mgr = SvmManager::detect(&spt_dir).unwrap();
        let mut config = mgr.config().clone();
        config.raids.raid_time = 45;
        mgr.save_preset("Default", &config).unwrap();
        assert!(mgr.is_dirty());
        assert_eq!(mgr.config().raids.raid_time, 45);

        // Reload from disk and verify persistence
        mgr.reload_from_disk().unwrap();
        assert_eq!(mgr.config().raids.raid_time, 45);
        assert!(mgr.is_dirty());
    }

    #[test]
    fn unknown_fields_preserved_on_save() {
        let tmp = TempDir::new().unwrap();
        let spt_dir = setup_svm_dir(&tmp);
        // Add unknown field to preset
        let preset_path =
            spt_dir.join("SPT/user/mods/[SVM] Server Value Modifier/Presets/Default.json");
        let mut raw: Value =
            serde_json::from_str(&fs::read_to_string(&preset_path).unwrap()).unwrap();
        raw.as_object_mut()
            .unwrap()
            .insert("FutureSection".into(), Value::String("hello".into()));
        fs::write(&preset_path, serde_json::to_string_pretty(&raw).unwrap()).unwrap();

        let mut mgr = SvmManager::detect(&spt_dir).unwrap();
        assert!(mgr.unknown_fields().contains(&"FutureSection".to_string()));

        // Save with no changes, verify unknown field survives
        let config = mgr.config().clone();
        mgr.save_preset("Default", &config).unwrap();
        let saved: Value =
            serde_json::from_str(&fs::read_to_string(&preset_path).unwrap()).unwrap();
        assert_eq!(saved.get("FutureSection").unwrap(), "hello");
    }

    #[test]
    fn preset_name_validation() {
        assert!(SvmManager::validate_preset_name("My Preset-1").is_ok());
        assert!(SvmManager::validate_preset_name("").is_err());
        assert!(SvmManager::validate_preset_name("../etc/passwd").is_err());
        assert!(SvmManager::validate_preset_name("a".repeat(65).as_str()).is_err());
        assert!(SvmManager::validate_preset_name("CON").is_err());
    }

    #[test]
    fn create_and_delete_presets() {
        let tmp = TempDir::new().unwrap();
        let spt_dir = setup_svm_dir(&tmp);
        let mut mgr = SvmManager::detect(&spt_dir).unwrap();
        mgr.create_preset("Hardcore").unwrap();
        assert_eq!(mgr.list_presets().len(), 2);
        mgr.delete_preset("Hardcore").unwrap();
        assert_eq!(mgr.list_presets().len(), 1);
    }

    #[test]
    fn cannot_delete_active_preset() {
        let tmp = TempDir::new().unwrap();
        let spt_dir = setup_svm_dir(&tmp);
        let mut mgr = SvmManager::detect(&spt_dir).unwrap();
        assert!(mgr.delete_preset("Default").is_err());
    }

    #[test]
    fn deleting_preset_alias_preserves_active_preset_and_loader() {
        use std::os::unix::fs::symlink;

        let tmp = TempDir::new().unwrap();
        let spt_dir = setup_svm_dir(&tmp);
        let svm = spt_dir.join("SPT/user/mods/[SVM] Server Value Modifier");
        let preset = svm.join("Presets/Default.json");
        let loader = svm.join(LOADER_FILE);
        let preset_before = fs::read(&preset).unwrap();
        let loader_before = fs::read(&loader).unwrap();
        symlink("Default.json", svm.join("Presets/Alias.json")).unwrap();
        symlink(
            "../Loader/loader.json",
            svm.join("Presets/LoaderAlias.json"),
        )
        .unwrap();
        let mut mgr = SvmManager::detect(&spt_dir).unwrap();
        mgr.delete_preset("Alias").unwrap();
        mgr.delete_preset("LoaderAlias").unwrap();
        assert_eq!(fs::read(&preset).unwrap(), preset_before);
        assert_eq!(fs::read(&loader).unwrap(), loader_before);
        assert!(!svm.join("Presets/Alias.json").exists());
        assert!(!svm.join("Presets/LoaderAlias.json").exists());
    }

    #[test]
    fn activating_preset_preserves_loader_mode() {
        use std::os::unix::fs::PermissionsExt;

        let tmp = TempDir::new().unwrap();
        let spt_dir = setup_svm_dir(&tmp);
        let loader = spt_dir.join("SPT/user/mods/[SVM] Server Value Modifier/Loader/loader.json");
        fs::set_permissions(&loader, fs::Permissions::from_mode(0o600)).unwrap();
        let mut mgr = SvmManager::detect(&spt_dir).unwrap();
        mgr.create_preset("Other").unwrap();
        mgr.set_active_preset("Other").unwrap();
        assert_eq!(
            fs::metadata(loader).unwrap().permissions().mode() & 0o777,
            0o600
        );
    }

    #[test]
    #[ignore = "requires setfacl and getfacl on an ACL-enabled filesystem"]
    fn activating_preset_preserves_loader_access_acl() {
        use std::process::Command;

        let tmp = TempDir::new().unwrap();
        let spt_dir = setup_svm_dir(&tmp);
        let loader = spt_dir.join("SPT/user/mods/[SVM] Server Value Modifier/Loader/loader.json");
        assert!(Command::new("setfacl")
            .arg("-m")
            .arg("u:12345:r--")
            .arg(&loader)
            .status()
            .unwrap()
            .success());
        let before = Command::new("getfacl")
            .arg("-cn")
            .arg(&loader)
            .output()
            .unwrap();
        assert!(before.status.success());
        let mut mgr = SvmManager::detect(&spt_dir).unwrap();
        mgr.create_preset("Other").unwrap();
        mgr.set_active_preset("Other").unwrap();
        let after = Command::new("getfacl")
            .arg("-cn")
            .arg(&loader)
            .output()
            .unwrap();
        assert!(after.status.success());
        assert_eq!(before.stdout, after.stdout);
    }

    #[test]
    #[ignore = "requires setfacl and getfacl on an ACL-enabled filesystem"]
    fn activation_does_not_add_parent_default_acl_to_existing_loader() {
        use std::process::Command;

        let tmp = TempDir::new().unwrap();
        let spt_dir = setup_svm_dir(&tmp);
        let loader = spt_dir.join("SPT/user/mods/[SVM] Server Value Modifier/Loader/loader.json");
        let before = Command::new("getfacl")
            .arg("-cn")
            .arg(&loader)
            .output()
            .unwrap();
        assert!(before.status.success());
        assert!(Command::new("setfacl")
            .args(["-d", "-m", "u:12345:r--"])
            .arg(loader.parent().unwrap())
            .status()
            .unwrap()
            .success());
        let mut mgr = SvmManager::detect(&spt_dir).unwrap();
        mgr.create_preset("Other").unwrap();
        mgr.set_active_preset("Other").unwrap();
        let after = Command::new("getfacl")
            .arg("-cn")
            .arg(&loader)
            .output()
            .unwrap();
        assert!(after.status.success());
        assert_eq!(before.stdout, after.stdout);
    }

    #[test]
    fn switch_active_preset() {
        let tmp = TempDir::new().unwrap();
        let spt_dir = setup_svm_dir(&tmp);
        let mut mgr = SvmManager::detect(&spt_dir).unwrap();
        mgr.create_preset("Alt").unwrap();
        mgr.set_active_preset("Alt").unwrap();
        assert_eq!(mgr.active_preset_name(), "Alt");
    }

    #[test]
    fn detect_inactive_preset_does_not_modify_files() {
        let tmp = TempDir::new().unwrap();
        let spt_dir = tmp.path().to_path_buf();
        let svm_dir = spt_dir.join("SPT/user/mods/[SVM] Server Value Modifier");
        fs::create_dir_all(svm_dir.join("Loader")).unwrap();
        let loader_path = svm_dir.join("Loader/loader.json");
        for selection in ["null", "\"null\"", "\"\""] {
            let loader = format!(r#"{{"CurrentlySelectedPreset": {selection}, "Other": 123}}"#);
            fs::write(&loader_path, &loader).unwrap();
            let mut mgr = SvmManager::detect(&spt_dir).expect("inactive SVM should be detected");
            assert_eq!(mgr.active_preset_name(), "");
            assert!(mgr.list_presets().is_empty());
            mgr.reload_from_disk().unwrap();
            assert_eq!(fs::read_to_string(&loader_path).unwrap(), loader);
            assert!(!svm_dir.join("Presets").exists());
        }
    }

    #[test]
    fn create_and_activate_preset_requires_explicit_activation() {
        let tmp = TempDir::new().unwrap();
        let spt_dir = tmp.path().to_path_buf();
        let svm_dir = spt_dir.join("SPT/user/mods/[SVM] Server Value Modifier");
        fs::create_dir_all(svm_dir.join("Loader")).unwrap();
        let loader_path = svm_dir.join("Loader/loader.json");
        let loader = r#"{"CurrentlySelectedPreset":"null","Other":123}"#;
        fs::write(&loader_path, loader).unwrap();
        let mut mgr = SvmManager::detect(&spt_dir).unwrap();
        mgr.create_preset("Custom").unwrap();
        assert_eq!(fs::read_to_string(&loader_path).unwrap(), loader);
        assert_eq!(mgr.active_preset_name(), "");
        mgr.set_active_preset("Custom").unwrap();
        let saved: Value =
            serde_json::from_str(&fs::read_to_string(&loader_path).unwrap()).unwrap();
        assert_eq!(saved["CurrentlySelectedPreset"], "Custom");
        assert_eq!(saved["Other"], 123);
        mgr.reload_from_disk().unwrap();
        assert_eq!(mgr.active_preset_name(), "Custom");
    }

    #[test]
    fn activation_marks_dirty_and_reload_preserves_it() {
        let tmp = TempDir::new().unwrap();
        let spt_dir = setup_svm_dir(&tmp);
        let mut mgr = SvmManager::detect(&spt_dir).unwrap();
        mgr.create_preset("Other").unwrap();
        assert!(!mgr.is_dirty());
        mgr.set_active_preset("Other").unwrap();
        assert!(mgr.is_dirty());
        mgr.reload_from_disk().unwrap();
        assert!(mgr.is_dirty());
        mgr.clear_dirty();
        mgr.set_active_preset("Other").unwrap();
        assert!(!mgr.is_dirty());
    }

    #[test]
    fn duplicate_preset() {
        let tmp = TempDir::new().unwrap();
        let spt_dir = setup_svm_dir(&tmp);
        let mut mgr = SvmManager::detect(&spt_dir).unwrap();
        mgr.duplicate_preset("Default", "Copy").unwrap();
        assert_eq!(mgr.list_presets().len(), 2);
        assert!(mgr.list_presets().contains(&"Copy".to_string()));
        // Duplicating from non-existent source fails
        assert!(mgr.duplicate_preset("NonExistent", "Another").is_err());
    }

    #[test]
    fn detect_case_insensitive_dir_name() {
        let tmp = TempDir::new().unwrap();
        let spt_dir = tmp.path().to_path_buf();
        let svm_dir = spt_dir.join("SPT/user/mods/[svm] server value modifier");
        fs::create_dir_all(svm_dir.join("Loader")).unwrap();
        fs::create_dir_all(svm_dir.join("Presets")).unwrap();
        let loader = serde_json::json!({"CurrentlySelectedPreset": "Default"});
        fs::write(svm_dir.join("Loader/loader.json"), loader.to_string()).unwrap();
        let config = SvmConfig::default();
        fs::write(
            svm_dir.join("Presets/Default.json"),
            serde_json::to_string_pretty(&config).unwrap(),
        )
        .unwrap();
        let mgr = SvmManager::detect(&spt_dir).expect("should detect SVM case-insensitively");
        assert_eq!(mgr.active_preset_name(), "Default");
    }
}
