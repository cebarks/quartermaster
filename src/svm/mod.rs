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
    active_preset: String,
    config: SvmConfig,
    raw_json: Value,
    unknown_fields: Vec<String>,
    available_presets: Vec<String>,
    dirty: bool,
}

impl SvmManager {
    /// Detect SVM installation and initialize manager.
    /// Returns None if SVM is not installed or detection fails.
    pub fn detect(spt_dir: &Path) -> Option<Self> {
        let svm_dir = crate::config::find_svm_dir(spt_dir)?;

        let loader_path = svm_dir.join(LOADER_FILE);
        if !loader_path.exists() {
            tracing::warn!("SVM installed but loader.json missing");
            return None;
        }

        // Read loader.json to get active preset
        let loader_content = std::fs::read_to_string(&loader_path).ok()?;
        // SVM is a .NET mod — files may have a UTF-8 BOM
        let loader_content = loader_content
            .strip_prefix('\u{feff}')
            .unwrap_or(&loader_content);
        let loader_json: Value = serde_json::from_str(loader_content).ok()?;
        let mut active_preset = loader_json
            .get("CurrentlySelectedPreset")
            .and_then(|v| v.as_str())
            .map(|s| s.to_string())
            .unwrap_or_default();

        // If preset is null/missing/empty, create Default preset
        let presets_dir = svm_dir.join(PRESETS_DIR);
        if active_preset.is_empty() || active_preset == "null" {
            active_preset = "Default".to_string();
            let default_config = SvmConfig::default();
            let default_path = presets_dir.join("Default.json");
            if !default_path.exists() {
                std::fs::create_dir_all(&presets_dir).ok()?;
                let json = serde_json::to_string_pretty(&default_config).ok()?;
                std::fs::write(&default_path, json).ok()?;
            }
            // Update loader.json
            let new_loader = serde_json::json!({"CurrentlySelectedPreset": "Default"});
            std::fs::write(
                &loader_path,
                serde_json::to_string_pretty(&new_loader).ok()?,
            )
            .ok()?;
        }

        // Scan available presets
        let available_presets = Self::scan_presets(&presets_dir).ok()?;

        // Load active preset
        let mut mgr = Self {
            svm_dir,
            active_preset: active_preset.clone(),
            config: SvmConfig::default(),
            raw_json: Value::Null,
            unknown_fields: Vec::new(),
            available_presets,
            dirty: false,
        };

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
        let preset_path = self
            .svm_dir
            .join(PRESETS_DIR)
            .join(format!("{}.json", name));
        let content = std::fs::read_to_string(&preset_path)
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
        // Serialize config to JSON
        let config_json = serde_json::to_value(config)?;

        // Deep merge into raw_json to preserve unknown fields
        let mut merged = self.raw_json.clone();
        Self::deep_merge(&mut merged, &config_json);

        // Write to file
        let preset_path = self
            .svm_dir
            .join(PRESETS_DIR)
            .join(format!("{}.json", name));
        let json_str = serde_json::to_string_pretty(&merged)?;
        std::fs::write(&preset_path, json_str)
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
        // Verify preset exists
        if !self.available_presets.contains(&name.to_string()) {
            bail!("Preset '{}' does not exist", name);
        }

        // Update loader.json
        let loader_path = self.svm_dir.join(LOADER_FILE);
        let loader_json = serde_json::json!({"CurrentlySelectedPreset": name});
        std::fs::write(&loader_path, serde_json::to_string_pretty(&loader_json)?)?;

        // Load the preset
        let (config, raw_json, unknown_fields) = self.load_preset_from_disk(name)?;
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
        let preset_path = self
            .svm_dir
            .join(PRESETS_DIR)
            .join(format!("{}.json", name));
        let default_config = SvmConfig::default();
        let json_str = serde_json::to_string_pretty(&default_config)?;
        std::fs::write(&preset_path, json_str)
            .with_context(|| format!("Failed to create preset '{}'", name))?;

        // Add to available presets
        self.available_presets.push(name.to_string());
        self.available_presets.sort();

        Ok(())
    }

    /// Duplicate a preset.
    pub fn duplicate_preset(&mut self, source: &str, dest: &str) -> Result<()> {
        Self::validate_preset_name(dest)?;

        // Check source exists
        if !self.available_presets.contains(&source.to_string()) {
            bail!("Source preset '{}' does not exist", source);
        }

        // Check destination doesn't exist
        if self.available_presets.contains(&dest.to_string()) {
            bail!("Preset '{}' already exists", dest);
        }

        // Read source and write to destination
        let source_path = self
            .svm_dir
            .join(PRESETS_DIR)
            .join(format!("{}.json", source));
        let dest_path = self
            .svm_dir
            .join(PRESETS_DIR)
            .join(format!("{}.json", dest));

        std::fs::copy(&source_path, &dest_path)
            .with_context(|| format!("Failed to duplicate preset '{}' to '{}'", source, dest))?;

        // Add to available presets
        self.available_presets.push(dest.to_string());
        self.available_presets.sort();

        Ok(())
    }

    /// Delete a preset.
    pub fn delete_preset(&mut self, name: &str) -> Result<()> {
        // Cannot delete active preset
        if name == self.active_preset {
            bail!("Cannot delete the currently active preset '{}'", name);
        }

        // Remove file
        let preset_path = self
            .svm_dir
            .join(PRESETS_DIR)
            .join(format!("{}.json", name));
        std::fs::remove_file(&preset_path)
            .with_context(|| format!("Failed to delete preset '{}'", name))?;

        // Remove from available presets
        self.available_presets.retain(|p| p != name);

        Ok(())
    }

    /// Reload from disk (discard any unsaved changes).
    pub fn reload_from_disk(&mut self) -> Result<()> {
        // Re-read loader.json
        let loader_path = self.svm_dir.join(LOADER_FILE);
        let loader_content = std::fs::read_to_string(&loader_path)?;
        let loader_json: Value = serde_json::from_str(&loader_content)?;
        let active_preset = loader_json
            .get("CurrentlySelectedPreset")
            .and_then(|v| v.as_str())
            .context("No active preset in loader.json")?
            .to_string();

        // Re-scan presets
        let presets_dir = self.svm_dir.join(PRESETS_DIR);
        self.available_presets = Self::scan_presets(&presets_dir)?;

        // Re-load active preset
        let (config, raw_json, unknown_fields) = self.load_preset_from_disk(&active_preset)?;
        self.active_preset = active_preset;
        self.config = config;
        self.raw_json = raw_json;
        self.unknown_fields = unknown_fields;
        self.dirty = false;

        Ok(())
    }

    /// Get the list of unknown field paths.
    pub fn unknown_fields(&self) -> &[String] {
        &self.unknown_fields
    }

    pub fn preset_path(&self, name: &str) -> PathBuf {
        self.svm_dir.join(PRESETS_DIR).join(format!("{name}.json"))
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
        assert!(!mgr.is_dirty());
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
    fn switch_active_preset() {
        let tmp = TempDir::new().unwrap();
        let spt_dir = setup_svm_dir(&tmp);
        let mut mgr = SvmManager::detect(&spt_dir).unwrap();
        mgr.create_preset("Alt").unwrap();
        mgr.set_active_preset("Alt").unwrap();
        assert_eq!(mgr.active_preset_name(), "Alt");
    }

    #[test]
    fn detect_creates_default_when_no_presets() {
        let tmp = TempDir::new().unwrap();
        let spt_dir = tmp.path().to_path_buf();
        let svm_dir = spt_dir.join("SPT/user/mods/[SVM] Server Value Modifier");
        fs::create_dir_all(svm_dir.join("Loader")).unwrap();
        fs::create_dir_all(svm_dir.join("Presets")).unwrap();
        // loader.json with null preset
        fs::write(
            svm_dir.join("Loader/loader.json"),
            r#"{"CurrentlySelectedPreset": null}"#,
        )
        .unwrap();
        let mgr = SvmManager::detect(&spt_dir).expect("should create default and detect");
        assert_eq!(mgr.active_preset_name(), "Default");
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
