use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};
use std::path::{Path, PathBuf};
use std::time::Duration;

use serde::Deserialize;

use crate::db::Database;

const CACHE_TTL: Duration = Duration::from_secs(15 * 60);

fn cache_key(mod_ids: &[i64], bundles_only: bool) -> String {
    let mut sorted = mod_ids.to_vec();
    sorted.sort_unstable();
    let mut hasher = DefaultHasher::new();
    sorted.hash(&mut hasher);
    bundles_only.hash(&mut hasher);
    format!("convoy-{:016x}.zip", hasher.finish())
}

fn is_fresh(path: &Path) -> bool {
    path.metadata()
        .and_then(|m| m.modified())
        .map(|mtime| mtime.elapsed().unwrap_or(Duration::MAX) < CACHE_TTL)
        .unwrap_or(false)
}

/// Returns the path to a cached ZIP of client-side files for the given mod IDs.
/// Builds the ZIP to disk on cache miss; serves from cache on hit (15-min TTL).
pub fn get_or_build_convoy_zip(
    db: &Database,
    spt_dir: &Path,
    mod_ids: &[i64],
    bundles_only: bool,
) -> anyhow::Result<PathBuf> {
    let cache_dir = spt_dir.join("quartermaster-cache/convoy");
    std::fs::create_dir_all(&cache_dir)?;

    let dest = cache_dir.join(cache_key(mod_ids, bundles_only));
    if is_fresh(&dest) {
        tracing::debug!(path = %dest.display(), "convoy zip cache hit");
        return Ok(dest);
    }

    let files = db.get_files_for_mod_ids(mod_ids)?;

    let bundles = discover_mod_bundles(spt_dir, &files);

    if !bundles_only
        && files
            .iter()
            .all(|f| f.file_path.starts_with("SPT/user/mods/"))
        && bundles.is_empty()
    {
        anyhow::bail!("no downloadable client files found for requested mod IDs");
    }

    let tmp = dest.with_extension("zip.tmp");
    build_convoy_zip_to_file(spt_dir, &files, &bundles, &tmp, bundles_only)?;
    std::fs::rename(&tmp, &dest)?;

    tracing::debug!(path = %dest.display(), "convoy zip built to disk");
    Ok(dest)
}

/// Remove all cached convoy ZIPs. Call when mods are installed/updated/removed.
pub fn clear_convoy_cache(spt_dir: &Path) {
    let cache_dir = spt_dir.join("quartermaster-cache/convoy");
    if let Err(e) = std::fs::remove_dir_all(&cache_dir) {
        if e.kind() != std::io::ErrorKind::NotFound {
            tracing::warn!(err = %e, "failed to clear convoy zip cache");
        }
    }
}

struct BundleFile {
    cache_path: String,
    source_path: PathBuf,
}

#[derive(Deserialize)]
struct BundleManifest {
    manifest: Option<Vec<BundleManifestEntry>>,
}

#[derive(Deserialize)]
struct BundleManifestEntry {
    key: String,
}

fn discover_mod_bundles(
    spt_dir: &Path,
    files: &[crate::db::mods::InstalledFile],
) -> Vec<BundleFile> {
    let mut mod_dirs: std::collections::HashSet<String> = std::collections::HashSet::new();
    for f in files {
        if let Some(rest) = f.file_path.strip_prefix("SPT/user/mods/") {
            if let Some(dir) = rest.split('/').next() {
                mod_dirs.insert(dir.to_string());
            }
        }
    }

    let mut bundles = Vec::new();
    for mod_dir in &mod_dirs {
        let bundles_json = spt_dir
            .join("SPT/user/mods")
            .join(mod_dir)
            .join("bundles.json");
        if !bundles_json.is_file() {
            continue;
        }

        let content = match std::fs::read_to_string(&bundles_json) {
            Ok(c) => c,
            Err(e) => {
                tracing::warn!(path = %bundles_json.display(), err = %e, "failed to read bundles.json");
                continue;
            }
        };

        let manifest: BundleManifest = match serde_json::from_str(&content) {
            Ok(m) => m,
            Err(e) => {
                tracing::warn!(path = %bundles_json.display(), err = %e, "failed to parse bundles.json");
                continue;
            }
        };

        let entries = match manifest.manifest {
            Some(e) => e,
            None => continue,
        };

        let bundles_dir = spt_dir.join("SPT/user/mods").join(mod_dir).join("bundles");
        for entry in entries {
            let source_path = bundles_dir.join(&entry.key);
            if !source_path.is_file() {
                tracing::warn!(key = %entry.key, mod_dir = %mod_dir, "bundle file missing, skipping");
                continue;
            }
            bundles.push(BundleFile {
                cache_path: format!("SPT/user/cache/bundles/{}", entry.key),
                source_path,
            });
        }
    }

    bundles
}

fn build_convoy_zip_to_file(
    spt_dir: &Path,
    files: &[crate::db::mods::InstalledFile],
    bundles: &[BundleFile],
    dest: &Path,
    bundles_only: bool,
) -> anyhow::Result<()> {
    use std::io::{BufWriter, Write};

    let file = BufWriter::new(std::fs::File::create(dest)?);
    let mut zip = zip::ZipWriter::new(file);
    let options = zip::write::SimpleFileOptions::default()
        .compression_method(zip::CompressionMethod::Deflated);

    if !bundles_only {
        for f in files {
            if f.file_path.starts_with("SPT/user/mods/") {
                continue;
            }
            if std::path::Path::new(&f.file_path).is_absolute()
                || f.file_path.split('/').any(|c| c == "..")
                || f.file_path.split('\\').any(|c| c == "..")
            {
                tracing::warn!(path = %f.file_path, "skipping file with unsafe path");
                continue;
            }
            let full_path = spt_dir.join(&f.file_path);
            match std::fs::read(&full_path) {
                Ok(data) => {
                    zip.start_file(&f.file_path, options)?;
                    zip.write_all(&data)?;
                }
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                    tracing::warn!(path = %f.file_path, "skipping missing file");
                    continue;
                }
                Err(e) => return Err(e.into()),
            }
        }
    }

    for b in bundles {
        match std::fs::read(&b.source_path) {
            Ok(data) => {
                zip.start_file(&b.cache_path, options)?;
                zip.write_all(&data)?;
            }
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                tracing::warn!(path = %b.cache_path, "skipping missing bundle file");
                continue;
            }
            Err(e) => return Err(e.into()),
        }
    }

    zip.finish()?;
    Ok(())
}
