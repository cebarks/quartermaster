use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};
use std::path::{Path, PathBuf};
use std::time::Duration;

use crate::db::Database;
use crate::web::mod_zip_cache::build_mod_zip_to_file;

const CACHE_TTL: Duration = Duration::from_secs(15 * 60);

fn cache_key(mod_ids: &[i64]) -> String {
    let mut sorted = mod_ids.to_vec();
    sorted.sort_unstable();
    let mut hasher = DefaultHasher::new();
    sorted.hash(&mut hasher);
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
) -> anyhow::Result<PathBuf> {
    let cache_dir = spt_dir.join("quartermaster-cache/convoy");
    std::fs::create_dir_all(&cache_dir)?;

    let dest = cache_dir.join(cache_key(mod_ids));
    if is_fresh(&dest) {
        tracing::debug!(path = %dest.display(), "convoy zip cache hit");
        return Ok(dest);
    }

    let files = db.get_files_for_mod_ids(mod_ids)?;
    if files
        .iter()
        .all(|f| f.file_path.starts_with("SPT/user/mods/"))
    {
        anyhow::bail!("no downloadable client files found for requested mod IDs");
    }

    let tmp = dest.with_extension("zip.tmp");
    build_mod_zip_to_file(spt_dir, &files, &tmp, true)?;
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
