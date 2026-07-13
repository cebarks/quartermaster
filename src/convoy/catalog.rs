use serde::Serialize;
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Duration;

use crate::config::ConvoyConfig;
use crate::db::Database;

#[derive(Debug, Clone, Serialize)]
pub struct Catalog {
    pub spt_version: String,
    pub quartermaster_version: String,
    pub groups: Vec<CatalogGroup>,
    pub exclusions: Vec<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct CatalogGroup {
    pub slug: String,
    pub name: String,
    pub tier: String,
    pub mods: Vec<CatalogMod>,
}

#[derive(Debug, Clone, Serialize)]
pub struct CatalogMod {
    pub id: i64,
    pub forge_id: Option<i64>,
    pub name: String,
    pub version: String,
    pub file_checksums: BTreeMap<String, String>,
}

fn get_client_file_checksums(
    db: &Database,
    m: &crate::db::mods::InstalledMod,
) -> anyhow::Result<BTreeMap<String, String>> {
    let files = db.get_files_for_mod_ids(&[m.id])?;
    Ok(files
        .iter()
        .filter(|f| f.file_path.starts_with("BepInEx/"))
        .filter_map(|f| {
            f.file_hash
                .as_ref()
                .map(|h| (f.file_path.clone(), h.clone()))
        })
        .collect())
}

pub fn generate_catalog(
    db: &Database,
    spt_dir: &Path,
    convoy_config: &ConvoyConfig,
) -> anyhow::Result<Catalog> {
    let spt_version = crate::spt::detect::read_spt_version(spt_dir)
        .map(|v| v.spt_version)
        .unwrap_or_else(|_| "unknown".to_string());

    let groups = db.list_groups()?;
    let all_mods = db.list_mods()?;

    let mut catalog_groups = Vec::new();

    // Build implicit "default" required group for ungrouped mods
    let ungrouped: Vec<_> = all_mods
        .iter()
        .filter(|m| m.group_id.is_none() && !m.disabled)
        .collect();
    let mut default_mods = Vec::new();
    for m in ungrouped {
        let checksums = get_client_file_checksums(db, m)?;
        if !checksums.is_empty() {
            default_mods.push(CatalogMod {
                id: m.id,
                forge_id: m.forge_mod_id,
                name: m.name.clone(),
                version: m.version.clone(),
                file_checksums: checksums,
            });
        }
    }
    if !default_mods.is_empty() {
        catalog_groups.push(CatalogGroup {
            slug: "default".to_string(),
            name: "Default".to_string(),
            tier: "required".to_string(),
            mods: default_mods,
        });
    }

    // Build group for each DB group (already sorted alphabetically by list_groups)
    for group in &groups {
        let group_mods: Vec<_> = all_mods
            .iter()
            .filter(|m| m.group_id == Some(group.id) && !m.disabled)
            .collect();

        let mut catalog_mods = Vec::new();
        for m in group_mods {
            let checksums = get_client_file_checksums(db, m)?;
            if !checksums.is_empty() {
                catalog_mods.push(CatalogMod {
                    id: m.id,
                    forge_id: m.forge_mod_id,
                    name: m.name.clone(),
                    version: m.version.clone(),
                    file_checksums: checksums,
                });
            }
        }
        if !catalog_mods.is_empty() {
            catalog_groups.push(CatalogGroup {
                slug: group.slug.clone(),
                name: group.name.clone(),
                tier: group.tier.clone(),
                mods: catalog_mods,
            });
        }
    }

    // Collect exclusions from config + runtime detection
    let mut exclusions = convoy_config.exclusions.clone();
    let runtime_exclusions = collect_runtime_exclusions(db, spt_dir)?;
    exclusions.extend(runtime_exclusions);
    exclusions.sort();
    exclusions.dedup();

    Ok(Catalog {
        spt_version,
        quartermaster_version: env!("QUMA_VERSION").to_string(),
        groups: catalog_groups,
        exclusions,
    })
}

// Runtime exclusion detection - ported from modsync.rs

fn collect_runtime_exclusions(db: &Database, spt_dir: &Path) -> anyhow::Result<Vec<String>> {
    let all_files = db.get_all_tracked_files()?;
    let archive_paths: std::collections::BTreeSet<String> = all_files
        .iter()
        .filter(|f| f.source == "archive")
        .map(|f| f.file_path.clone())
        .collect();

    let mod_dirs: std::collections::BTreeSet<String> = all_files
        .iter()
        .filter(|f| f.file_path.starts_with("BepInEx/plugins/"))
        .filter_map(|f| {
            let rest = f.file_path.strip_prefix("BepInEx/plugins/")?;
            let dir = rest.split('/').next()?;
            Some(format!("BepInEx/plugins/{}", dir))
        })
        .collect();

    let mut exclusions: std::collections::BTreeSet<String> = std::collections::BTreeSet::new();

    for mod_dir_rel in mod_dirs {
        let mod_dir_abs = spt_dir.join(&mod_dir_rel);
        if !mod_dir_abs.is_dir() {
            continue;
        }
        exclude_untracked_recursive(&mod_dir_abs, spt_dir, &archive_paths, &mut exclusions);
    }

    Ok(exclusions.into_iter().collect())
}

/// Walk `dir` and add exclusions for files/subdirectories not in `archive_paths`.
/// If a subdirectory contains zero archive files, exclude it wholesale instead of
/// listing every file individually.
fn exclude_untracked_recursive(
    dir: &Path,
    spt_dir: &Path,
    archive_paths: &std::collections::BTreeSet<String>,
    exclusions: &mut std::collections::BTreeSet<String>,
) {
    let entries = match std::fs::read_dir(dir) {
        Ok(e) => e,
        Err(_) => return,
    };

    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_symlink() {
            continue;
        }

        if path.is_dir() {
            let dir_prefix = path
                .strip_prefix(spt_dir)
                .ok()
                .and_then(|r| r.to_str())
                .map(|r| format!("{r}/"));

            let has_archive = dir_prefix.as_ref().is_some_and(|prefix| {
                archive_paths
                    .range::<str, _>((
                        std::ops::Bound::Included(prefix.as_str()),
                        std::ops::Bound::Unbounded,
                    ))
                    .next()
                    .is_some_and(|p| p.starts_with(prefix.as_str()))
            });

            if has_archive {
                exclude_untracked_recursive(&path, spt_dir, archive_paths, exclusions);
            } else if let Some(prefix) = dir_prefix {
                exclusions.insert(prefix.trim_end_matches('/').to_string());
            }
        } else if let Ok(rel) = path.strip_prefix(spt_dir) {
            if let Some(rel_str) = rel.to_str() {
                if !archive_paths.contains(rel_str) {
                    exclusions.insert(rel_str.to_string());
                }
            }
        }
    }
}

#[derive(Clone)]
pub struct CatalogCache {
    inner: Arc<CatalogCacheInner>,
}

struct CatalogCacheInner {
    spt_dir: PathBuf,
    db: Arc<parking_lot::Mutex<Database>>,
    config: Arc<parking_lot::RwLock<crate::config::Config>>,
    cache_path: PathBuf,
    tmp_path: PathBuf,
    rehash_marker_path: PathBuf,
    rebuilding: AtomicBool,
    dirty: AtomicBool,
}

/// RAII guard that resets `rebuilding` to false on drop.
struct ResetOnDrop<'a>(&'a AtomicBool);
impl Drop for ResetOnDrop<'_> {
    fn drop(&mut self) {
        self.0.store(false, Ordering::Release);
    }
}

impl CatalogCache {
    pub fn new(
        spt_dir: PathBuf,
        db: Arc<parking_lot::Mutex<Database>>,
        config: Arc<parking_lot::RwLock<crate::config::Config>>,
    ) -> Self {
        let cache_dir = spt_dir.join("quartermaster-cache");
        std::fs::create_dir_all(&cache_dir).ok();
        Self {
            inner: Arc::new(CatalogCacheInner {
                spt_dir,
                db,
                config,
                cache_path: cache_dir.join("convoy-catalog.json"),
                tmp_path: cache_dir.join("convoy-catalog.json.tmp"),
                rehash_marker_path: cache_dir.join("convoy-last-rehash"),
                rebuilding: AtomicBool::new(false),
                dirty: AtomicBool::new(false),
            }),
        }
    }

    /// Returns (file_path, etag) if cached catalog exists.
    pub fn get(&self) -> Option<(PathBuf, String)> {
        if self.inner.cache_path.exists() {
            let metadata = std::fs::metadata(&self.inner.cache_path).ok()?;
            let modified = metadata.modified().ok()?;
            let etag = format!(
                "\"{:x}\"",
                modified
                    .duration_since(std::time::UNIX_EPOCH)
                    .unwrap_or_default()
                    .as_millis()
            );
            Some((self.inner.cache_path.clone(), etag))
        } else {
            None
        }
    }

    /// Trigger a background rebuild. Coalesces concurrent calls.
    pub fn invalidate(&self) {
        if self
            .inner
            .rebuilding
            .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
            .is_ok()
        {
            let cache = self.clone();
            tokio::task::spawn_blocking(move || {
                cache.inner.do_rebuild(&cache);
            });
        } else {
            self.inner.dirty.store(true, Ordering::Release);
        }
    }

    pub fn clear(&self) {
        let _ = std::fs::remove_file(&self.inner.cache_path);
    }

    /// Force rehash all tracked client files from disk, then rebuild catalog.
    pub fn force_rehash(&self) {
        let db = self.inner.db.lock();
        let updated = rehash_client_files(&db, &self.inner.spt_dir, true);
        drop(db);
        tracing::info!(updated, "force rehash complete");
        self.invalidate();
    }

    /// Synchronous rebuild for tests. Guarded by compare_exchange.
    #[cfg(test)]
    pub fn rebuild_sync(&self) {
        if self
            .inner
            .rebuilding
            .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
            .is_ok()
        {
            self.inner.do_rebuild(self);
        }
    }
}

impl CatalogCacheInner {
    fn do_rebuild(&self, cache: &CatalogCache) {
        let _guard = ResetOnDrop(&self.rebuilding);

        if let Err(e) = self.build_cache() {
            tracing::error!(err = %e, "failed to rebuild convoy catalog cache");
        }

        if self.dirty.swap(false, Ordering::AcqRel) {
            cache.invalidate();
        }
    }

    fn build_cache(&self) -> anyhow::Result<()> {
        self.rehash_if_stale();

        let convoy_config = self
            .config
            .read()
            .convoy
            .as_ref()
            .cloned()
            .unwrap_or_default();

        let db = self.db.lock();
        let catalog = generate_catalog(&db, &self.spt_dir, &convoy_config)?;
        drop(db);

        let json = serde_json::to_string_pretty(&catalog)?;
        std::fs::write(&self.tmp_path, &json)?;
        std::fs::rename(&self.tmp_path, &self.cache_path)?;

        tracing::info!("rebuilt convoy catalog cache");
        Ok(())
    }

    /// Rehash client files if the last rehash was more than 30 minutes ago.
    fn rehash_if_stale(&self) {
        let stale = self
            .rehash_marker_path
            .metadata()
            .and_then(|m| m.modified())
            .map(|mtime| mtime.elapsed().unwrap_or(Duration::MAX) > Duration::from_secs(30 * 60))
            .unwrap_or(true);

        if !stale {
            return;
        }

        let db = self.db.lock();
        let updated = rehash_client_files(&db, &self.spt_dir, false);
        drop(db);

        // Touch marker regardless of whether anything changed
        std::fs::write(&self.rehash_marker_path, b"").ok();

        if updated > 0 {
            tracing::info!(updated, "rehashed stale convoy file checksums");
        }
    }
}

const REHASH_STALE_SECS: u64 = 30 * 60;

/// Rehash all tracked BepInEx files from disk, updating DB where the hash differs.
/// If `force` is true, rehash everything; otherwise only files whose on-disk mtime
/// is newer than `REHASH_STALE_SECS`.
/// Returns the number of DB rows updated.
fn rehash_client_files(db: &Database, spt_dir: &Path, force: bool) -> usize {
    let files = match db.get_all_tracked_files() {
        Ok(f) => f,
        Err(e) => {
            tracing::error!(err = %e, "failed to list tracked files for rehash");
            return 0;
        }
    };

    let mut updated = 0;
    for file in &files {
        if !file.file_path.starts_with("BepInEx/") {
            continue;
        }

        let abs_path = spt_dir.join(&file.file_path);
        if !abs_path.is_file() {
            continue;
        }

        // Skip files not modified recently unless forced
        if !force {
            let dominated_by_mtime = abs_path
                .metadata()
                .and_then(|m| m.modified())
                .map(|mtime| {
                    mtime.elapsed().unwrap_or(Duration::MAX)
                        < Duration::from_secs(REHASH_STALE_SECS)
                })
                .unwrap_or(false);
            if !dominated_by_mtime {
                continue;
            }
        }

        let disk_hash = match crate::spt::mods::compute_file_hash(&abs_path) {
            Ok(h) => h,
            Err(e) => {
                tracing::warn!(path = %file.file_path, err = %e, "failed to hash file during rehash");
                continue;
            }
        };

        let db_hash = file.file_hash.as_deref().unwrap_or("");
        if disk_hash != db_hash {
            let size = abs_path.metadata().map(|m| m.len() as i64).ok();
            if let Err(e) = db.update_file_hash(file.id, &disk_hash, size) {
                tracing::warn!(path = %file.file_path, err = %e, "failed to update file hash");
            } else {
                tracing::debug!(path = %file.file_path, "updated stale file hash");
                updated += 1;
            }
        }
    }

    updated
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::Database;
    use tempfile::TempDir;

    fn setup() -> (TempDir, Database) {
        let dir = TempDir::new().unwrap();
        let db_path = dir.path().join("test.db");
        let db = Database::open(&db_path).unwrap();
        (dir, db)
    }

    #[test]
    fn empty_catalog() {
        let (dir, db) = setup();
        let config = ConvoyConfig::default();
        let catalog = generate_catalog(&db, dir.path(), &config).unwrap();
        assert!(catalog.groups.is_empty());
        assert!(catalog.exclusions.is_empty());
        assert_eq!(catalog.spt_version, "unknown");
    }

    #[test]
    fn ungrouped_mods_go_to_default_group() {
        let (dir, db) = setup();
        let mod_id = db
            .insert_mod(
                Some(1234),
                Some(1),
                "TestMod",
                Some("testmod"),
                "1.0.0",
                "forge",
                None,
            )
            .unwrap();
        db.insert_file(
            mod_id,
            "BepInEx/plugins/TestMod/test.dll",
            Some("abc123"),
            Some(1024),
        )
        .unwrap();

        let config = ConvoyConfig::default();
        let catalog = generate_catalog(&db, dir.path(), &config).unwrap();

        assert_eq!(catalog.groups.len(), 1);
        assert_eq!(catalog.groups[0].slug, "default");
        assert_eq!(catalog.groups[0].tier, "required");
        assert_eq!(catalog.groups[0].mods.len(), 1);
        assert_eq!(catalog.groups[0].mods[0].name, "TestMod");
    }

    #[test]
    fn grouped_mods_appear_in_their_group() {
        let (dir, db) = setup();
        let group_id = db
            .insert_group("Cosmetics", "cosmetics", "optional", false)
            .unwrap();
        let mod_id = db
            .insert_mod(
                Some(1234),
                Some(1),
                "PrettyMod",
                Some("prettymod"),
                "1.0.0",
                "forge",
                None,
            )
            .unwrap();
        db.set_mod_group(mod_id, Some(group_id)).unwrap();
        db.insert_file(
            mod_id,
            "BepInEx/plugins/PrettyMod/pretty.dll",
            Some("def456"),
            Some(2048),
        )
        .unwrap();

        let config = ConvoyConfig::default();
        let catalog = generate_catalog(&db, dir.path(), &config).unwrap();

        assert_eq!(catalog.groups.len(), 1);
        assert_eq!(catalog.groups[0].slug, "cosmetics");
        assert_eq!(catalog.groups[0].tier, "optional");
    }

    #[test]
    fn disabled_mods_excluded_from_catalog() {
        let (dir, db) = setup();
        let mod_id = db
            .insert_mod(
                Some(1234),
                Some(1),
                "DisabledMod",
                Some("disabled"),
                "1.0.0",
                "forge",
                None,
            )
            .unwrap();
        db.insert_file(
            mod_id,
            "BepInEx/plugins/DisabledMod/d.dll",
            Some("abc"),
            Some(100),
        )
        .unwrap();
        db.set_mod_disabled(mod_id, true).unwrap();

        let config = ConvoyConfig::default();
        let catalog = generate_catalog(&db, dir.path(), &config).unwrap();
        assert!(catalog.groups.is_empty());
    }

    #[test]
    fn server_only_mods_excluded_from_catalog() {
        let (dir, db) = setup();
        let mod_id = db
            .insert_mod(
                Some(1234),
                Some(1),
                "ServerMod",
                Some("servermod"),
                "1.0.0",
                "forge",
                None,
            )
            .unwrap();
        // Only has SPT/user/mods/ files, no BepInEx/ files
        db.insert_file(
            mod_id,
            "SPT/user/mods/ServerMod/mod.js",
            Some("abc"),
            Some(100),
        )
        .unwrap();

        let config = ConvoyConfig::default();
        let catalog = generate_catalog(&db, dir.path(), &config).unwrap();
        assert!(catalog.groups.is_empty());
    }

    #[test]
    fn config_exclusions_included() {
        let (dir, db) = setup();
        let config = ConvoyConfig {
            enabled: true,
            exclusions: vec!["BepInEx/plugins/SAIN/BotTypes.json".to_string()],
        };
        let catalog = generate_catalog(&db, dir.path(), &config).unwrap();
        assert!(catalog
            .exclusions
            .contains(&"BepInEx/plugins/SAIN/BotTypes.json".to_string()));
    }
}
