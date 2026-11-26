#[cfg(test)]
use serde::Serialize;
#[cfg(test)]
use std::collections::BTreeMap;
use std::path::Path;
#[cfg(test)]
use std::time::Duration;

#[cfg(test)]
use crate::config::ConvoyConfig;
#[cfg(test)]
use crate::db::Database;

#[cfg(test)]
#[derive(Debug, Clone, Serialize)]
pub struct Catalog {
    pub spt_version: String,
    pub quartermaster_version: String,
    pub groups: Vec<CatalogGroup>,
    pub exclusions: Vec<String>,
}

#[cfg(test)]
#[derive(Debug, Clone, Serialize)]
pub struct CatalogGroup {
    pub slug: String,
    pub name: String,
    pub tier: String,
    pub mods: Vec<CatalogMod>,
}

#[cfg(test)]
#[derive(Debug, Clone, Serialize)]
pub struct CatalogMod {
    pub id: i64,
    pub forge_id: Option<i64>,
    pub guid: Option<String>,
    pub name: String,
    pub version: String,
    pub file_checksums: BTreeMap<String, String>,
    pub bundle_checksums: BTreeMap<String, String>,
}

#[cfg(test)]
fn resolve_guid(
    m: &crate::db::mods::InstalledMod,
    spt_dir: &Path,
    db: &Database,
) -> Option<String> {
    if m.guid.is_some() {
        return m.guid.clone();
    }
    let files = db.get_files_for_mod_ids(&[m.id]).ok()?;
    let mod_dir = files.iter().find_map(|f| {
        crate::spt::layout::server_mod_suffix(&f.file_path).and_then(|rest| rest.split('/').next())
    })?;
    let pkg_path = crate::spt::layout::runtime_dir(spt_dir)
        .ok()?
        .join("user/mods")
        .join(mod_dir)
        .join("package.json");
    let content = std::fs::read_to_string(&pkg_path).ok()?;
    package_guid(content.as_bytes())
}

pub(super) fn package_guid(reader: impl std::io::Read) -> Option<String> {
    let v: serde_json::Value = serde_json::from_reader(reader).ok()?;
    v.get("name")
        .or_else(|| v.get("uid"))
        .and_then(|n| n.as_str())
        .map(|s| s.to_string())
}

/// Canonical portable client paths. Managed dependencies are direct DLL children,
/// never a recursive export of the game's Managed directory.
pub(crate) fn is_client_path(path: &str) -> bool {
    if path.starts_with("BepInEx/config/Convoy/") {
        return false;
    }
    portable_path(path) && has_client_namespace(path)
}

pub(super) fn has_client_namespace(path: &str) -> bool {
    path.starts_with("BepInEx/")
        || path
            .strip_prefix("EscapeFromTarkov_Data/Managed/")
            .is_some_and(|name| !name.contains('/') && name.ends_with(".dll") && name.len() > 4)
}

pub(super) fn portable_path(path: &str) -> bool {
    !path.contains(['\\', ':', '\0', '<', '>', '"', '|', '?', '*'])
        && !path.split('/').any(|p| {
            p.is_empty()
                || p == "."
                || p == ".."
                || p.ends_with(['.', ' '])
                || p.chars().any(char::is_control)
                || matches!(
                    p.split('.')
                        .next()
                        .unwrap_or("")
                        .to_ascii_uppercase()
                        .as_str(),
                    "CON"
                        | "PRN"
                        | "AUX"
                        | "NUL"
                        | "COM1"
                        | "COM2"
                        | "COM3"
                        | "COM4"
                        | "COM5"
                        | "COM6"
                        | "COM7"
                        | "COM8"
                        | "COM9"
                        | "LPT1"
                        | "LPT2"
                        | "LPT3"
                        | "LPT4"
                        | "LPT5"
                        | "LPT6"
                        | "LPT7"
                        | "LPT8"
                        | "LPT9"
                )
        })
}

pub(super) fn eligible_client_file(file: &crate::db::mods::InstalledFile) -> bool {
    is_client_path(&file.file_path)
        && (file.file_path.starts_with("BepInEx/") || file.source == "archive")
}

#[cfg(test)]
pub(super) fn validate_client_inventory(
    files: &[crate::db::mods::InstalledFile],
) -> anyhow::Result<()> {
    for file in files.iter().filter(|f| eligible_client_file(f)) {
        anyhow::ensure!(
            file.file_hash.is_some(),
            "missing checksum for Convoy client file {}; rehash the package before publishing",
            file.file_path
        );
    }
    Ok(())
}

#[cfg(test)]
pub(super) fn client_file_checksum(file: &crate::db::mods::InstalledFile) -> Option<&str> {
    if eligible_client_file(file) {
        file.file_hash.as_deref()
    } else {
        None
    }
}

#[cfg(test)]
fn get_client_file_checksums(
    db: &Database,
    m: &crate::db::mods::InstalledMod,
) -> anyhow::Result<BTreeMap<String, String>> {
    let files = db.get_files_for_mod_ids(&[m.id])?;
    validate_client_inventory(&files)?;
    Ok(files
        .iter()
        .filter_map(|f| client_file_checksum(f).map(|hash| (f.file_path.clone(), hash.to_owned())))
        .collect())
}

#[cfg(test)]
fn get_bundle_checksums(spt_dir: &Path, db: &Database, mod_id: i64) -> BTreeMap<String, String> {
    let files = match db.get_files_for_mod_ids(&[mod_id]) {
        Ok(f) => f,
        Err(e) => {
            tracing::warn!(mod_id, err = %e, "failed to query files for bundle discovery");
            return BTreeMap::new();
        }
    };

    let mut checksums = BTreeMap::new();
    for bundle in super::bundles::discover_mod_bundles(spt_dir, &files) {
        match bundle
            .open()
            .and_then(crate::spt::mods::compute_reader_hash)
        {
            Ok(hash) => {
                checksums.insert(bundle.cache_path, hash);
            }
            Err(e) => {
                tracing::warn!(path = %bundle.cache_path, err = %e, "failed to hash bundle file, skipping");
            }
        }
    }
    checksums
}

#[cfg(test)]
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
            let bundle_checksums = get_bundle_checksums(spt_dir, db, m.id);
            let guid = resolve_guid(m, spt_dir, db);
            default_mods.push(CatalogMod {
                id: m.id,
                forge_id: m.forge_mod_id,
                guid,
                name: m.name.clone(),
                version: m.version.clone(),
                file_checksums: checksums,
                bundle_checksums,
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
                let bundle_checksums = get_bundle_checksums(spt_dir, db, m.id);
                let guid = resolve_guid(m, spt_dir, db);
                catalog_mods.push(CatalogMod {
                    id: m.id,
                    forge_id: m.forge_mod_id,
                    guid,
                    name: m.name.clone(),
                    version: m.version.clone(),
                    file_checksums: checksums,
                    bundle_checksums,
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

pub(super) fn scoped_runtime_exclusions(
    scope: &crate::spt::scoped::ScopedDir,
    files: &[crate::db::mods::InstalledFile],
) -> anyhow::Result<Vec<String>> {
    use std::collections::BTreeSet;
    let tracked: BTreeSet<_> = files
        .iter()
        .filter(|f| f.source == "archive")
        .map(|f| f.file_path.as_str())
        .collect();
    let roots: BTreeSet<_> = files
        .iter()
        .filter(|f| eligible_client_file(f))
        .filter_map(|f| f.file_path.strip_prefix("BepInEx/plugins/"))
        .filter_map(|p| {
            p.split_once('/')
                .map(|(dir, _)| format!("BepInEx/plugins/{dir}"))
        })
        .collect();
    let mut exclusions = Vec::new();
    let mut dirs: Vec<_> = roots.into_iter().collect();
    while let Some(dir) = dirs.pop() {
        let relative = Path::new(&dir);
        // Existing directory links are not a source of automatic exclusions.
        if scope.resolve(relative)? != relative
            || !scope.entry_type(relative)?.is_some_and(|ty| ty.is_dir())
        {
            continue;
        }
        for name in scope.file_names(relative)? {
            let Some(name) = name.to_str() else {
                continue;
            };
            let path = format!("{dir}/{name}");
            if !super::manifest::portable_path(&path) {
                continue;
            }
            let Some(ty) = scope.entry_type(Path::new(&path))? else {
                continue;
            };
            if ty.is_dir() {
                let prefix = format!("{path}/");
                if tracked
                    .range(prefix.as_str()..)
                    .next()
                    .is_some_and(|p| p.starts_with(&prefix))
                {
                    dirs.push(path);
                } else {
                    exclusions.push(path);
                }
            } else if ty.is_file() && !tracked.contains(path.as_str()) {
                exclusions.push(path);
            }
        }
    }
    Ok(exclusions)
}

#[cfg(test)]
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
#[cfg(test)]
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

#[cfg(test)]
const REHASH_STALE_SECS: u64 = 30 * 60;

/// Rehash eligible tracked client files from scoped overlay layers, updating changed hashes.
/// If `force` is true, rehash everything; otherwise only files whose on-disk mtime
/// is newer than `REHASH_STALE_SECS`.
/// Returns the number of DB rows updated.
#[cfg(test)]
fn rehash_client_files(db: &Database, dirs: &crate::dirs::QumaDirs, force: bool) -> usize {
    let layers = match crate::spt::layered::LayeredFiles::open(dirs) {
        Ok(layers) => layers,
        Err(e) => {
            tracing::warn!(err = %e, "cannot open client layers for rehash");
            return 0;
        }
    };
    let files = match db.get_all_tracked_files() {
        Ok(f) => f,
        Err(e) => {
            tracing::error!(err = %e, "failed to list tracked files for rehash");
            return 0;
        }
    };

    let mut updated = 0;
    for file in &files {
        if !is_client_path(&file.file_path)
            || (!file.file_path.starts_with("BepInEx/") && file.source != "archive")
        {
            continue;
        }

        let source = match layers.read(Path::new(&file.file_path), true) {
            Ok(source) => source,
            Err(_) => continue,
        };
        let metadata = source.metadata().ok();

        // Skip files not modified recently unless forced
        if !force {
            let dominated_by_mtime = metadata
                .as_ref()
                .and_then(|m| m.modified().ok())
                .map(|mtime| {
                    mtime.elapsed().unwrap_or(Duration::MAX)
                        < Duration::from_secs(REHASH_STALE_SECS)
                })
                .unwrap_or(false);
            if !dominated_by_mtime {
                continue;
            }
        }

        let disk_hash = match crate::spt::mods::compute_reader_hash(source) {
            Ok(h) => h,
            Err(e) => {
                tracing::warn!(path = %file.file_path, err = %e, "failed to hash file during rehash");
                continue;
            }
        };

        let db_hash = file.file_hash.as_deref().unwrap_or("");
        if disk_hash != db_hash {
            let size = metadata.map(|m| m.len() as i64);
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
    #[cfg(test)]
    use crate::db::Database;
    use crate::dirs::QumaDirs;
    use tempfile::TempDir;

    fn setup() -> (TempDir, Database) {
        let dir = TempDir::new().unwrap();
        let db_path = dir.path().join("test.db");
        let db = Database::open(&db_path).unwrap();
        (dir, db)
    }

    #[test]
    fn tracked_managed_dependencies_are_published_and_rehashed() {
        let (dir, db) = setup();
        let id = db
            .insert_mod(None, None, "Dependency", None, "1", "local", None, None)
            .unwrap();
        let path = "EscapeFromTarkov_Data/Managed/Unity.VectorGraphics.dll";
        let absolute = dir.path().join(path);
        std::fs::create_dir_all(absolute.parent().unwrap()).unwrap();
        std::fs::write(&absolute, b"synthetic dependency").unwrap();
        db.insert_file(id, path, Some("stale"), None).unwrap();
        assert_eq!(
            rehash_client_files(&db, &QumaDirs::from_legacy(dir.path().to_path_buf()), true),
            1
        );
        let catalog = generate_catalog(&db, dir.path(), &ConvoyConfig::default()).unwrap();
        assert_eq!(
            catalog.groups[0].mods[0].file_checksums[path],
            crate::spt::mods::compute_file_hash(&absolute).unwrap()
        );
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

    #[cfg(unix)]
    #[test]
    fn bundle_exports_reject_unconfined_sources() {
        for runtime in ["SPT_Runtime", "SPT"] {
            let (dir, db) = setup();
            let id = db
                .insert_mod(None, None, "Test", None, "1", "local", None, None)
                .unwrap();
            let outside = tempfile::NamedTempFile::new().unwrap();
            std::fs::write(outside.path(), b"secret").unwrap();
            let base = dir.path().join(runtime);
            let mod_dir = base.join("user/mods/Test");
            std::fs::create_dir_all(mod_dir.join("bundles")).unwrap();
            std::fs::create_dir_all(base.join("SPT_Data/configs")).unwrap();
            std::fs::write(base.join("SPT_Data/configs/core.json"), b"secret").unwrap();
            std::os::unix::fs::symlink(outside.path(), mod_dir.join("bundles/outside.bundle"))
                .unwrap();
            std::fs::write(
                mod_dir.join("bundles.json"),
                serde_json::to_vec(&serde_json::json!({"manifest": [
                    {"key": outside.path().to_str().unwrap()},
                    {"key": "../../../../SPT_Data/configs/core.json"},
                    {"key": "outside.bundle"}
                ]}))
                .unwrap(),
            )
            .unwrap();
            db.insert_file(
                id,
                &format!("{runtime}/user/mods/Test/bundles.json"),
                None,
                None,
            )
            .unwrap();
            assert!(get_bundle_checksums(dir.path(), &db, id).is_empty());
        }
    }

    #[cfg(unix)]
    #[test]
    fn bundle_exports_preserve_confined_symlinks() {
        for runtime in ["SPT_Runtime", "SPT"] {
            let (dir, db) = setup();
            let id = db
                .insert_mod(None, None, "Test", None, "1", "local", None, None)
                .unwrap();
            let mod_dir = dir.path().join(runtime).join("user/mods/Test");
            std::fs::create_dir_all(mod_dir.join("bundles")).unwrap();
            let target = mod_dir.join("bundles/target.bundle");
            std::fs::write(&target, b"allowed bundle").unwrap();
            std::os::unix::fs::symlink("target.bundle", mod_dir.join("bundles/relative.bundle"))
                .unwrap();
            std::os::unix::fs::symlink(&target, mod_dir.join("bundles/absolute.bundle")).unwrap();
            std::fs::write(
                mod_dir.join("bundles.json"),
                br#"{"manifest":[{"key":"relative.bundle"},{"key":"absolute.bundle"}]}"#,
            )
            .unwrap();
            db.insert_file(
                id,
                &format!("{runtime}/user/mods/Test/bundles.json"),
                None,
                None,
            )
            .unwrap();
            assert_eq!(get_bundle_checksums(dir.path(), &db, id).len(), 2);
        }
    }

    #[test]
    fn bundle_exports_follow_the_client_runtime_layout() {
        for (runtime, expected) in [
            ("SPT_Runtime", "SPT_Runtime/user/cache/bundles/test.bundle"),
            ("SPT", "SPT/user/cache/bundles/test.bundle"),
        ] {
            let (dir, db) = setup();
            let id = db
                .insert_mod(
                    Some(1234),
                    Some(1),
                    "Test",
                    None,
                    "1.0",
                    "forge",
                    None,
                    None,
                )
                .unwrap();
            let relative = format!("{runtime}/user/mods/Test/bundles.json");
            let mod_dir = dir.path().join(runtime).join("user/mods/Test");
            std::fs::create_dir_all(mod_dir.join("bundles")).unwrap();
            std::fs::write(
                mod_dir.join("bundles.json"),
                br#"{"manifest":[{"key":"test.bundle"}]}"#,
            )
            .unwrap();
            std::fs::write(mod_dir.join("bundles/test.bundle"), b"bundle content").unwrap();
            db.insert_file(id, &relative, None, None).unwrap();
            let checksums = get_bundle_checksums(dir.path(), &db, id);
            assert_eq!(
                checksums.keys().map(String::as_str).collect::<Vec<_>>(),
                vec![expected]
            );
        }
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
            ..ConvoyConfig::default()
        };
        let catalog = generate_catalog(&db, dir.path(), &config).unwrap();
        assert!(catalog
            .exclusions
            .contains(&"BepInEx/plugins/SAIN/BotTypes.json".to_string()));
    }
}
