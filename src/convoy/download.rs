use std::collections::hash_map::DefaultHasher;
use std::fs::File;
use std::hash::{Hash, Hasher};
use std::io::{BufWriter, Write};
use std::path::{Path, PathBuf};
use std::time::Duration;

use serde::Deserialize;

use crate::db::mods::InstalledFile;
use crate::db::Database;
use crate::dirs::QumaDirs;
use crate::spt::layered::LayeredFiles;

use super::catalog::client_file_checksum;

const CACHE_TTL: Duration = Duration::from_secs(15 * 60);

fn cache_key(mod_ids: &[i64], bundles_only: bool) -> String {
    let mut sorted = mod_ids.to_vec();
    sorted.sort_unstable();
    let mut hasher = DefaultHasher::new();
    sorted.hash(&mut hasher);
    bundles_only.hash(&mut hasher);
    // Older cache keys can contain incomplete archives built from the base tree alone.
    format!("convoy-v2-{:016x}.zip", hasher.finish())
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
    dirs: &QumaDirs,
    mod_ids: &[i64],
    bundles_only: bool,
) -> anyhow::Result<PathBuf> {
    let cache_dir = dirs.spt_server.join("quartermaster-cache/convoy");
    std::fs::create_dir_all(&cache_dir)?;

    let dest = cache_dir.join(cache_key(mod_ids, bundles_only));
    if is_fresh(&dest) {
        tracing::debug!(path = %dest.display(), "convoy zip cache hit");
        return Ok(dest);
    }

    let files = db.get_files_for_mod_ids(mod_ids)?;

    let bundles = discover_mod_bundles(&dirs.spt_server, &files);

    if !bundles_only
        && files
            .iter()
            .all(|f| crate::spt::layout::is_server_path(&f.file_path))
        && bundles.is_empty()
    {
        anyhow::bail!("no downloadable client files found for requested mod IDs");
    }

    let mut tmp = tempfile::NamedTempFile::new_in(&cache_dir)?;
    build_convoy_zip(dirs, &files, &bundles, tmp.as_file_mut(), bundles_only)?;
    tmp.persist(&dest)?;

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

pub(crate) struct BundleFile {
    pub(crate) cache_path: String,
    source_path: PathBuf,
    scope: std::sync::Arc<crate::spt::scoped::ScopedDir>,
}

impl BundleFile {
    pub(crate) fn open(&self) -> anyhow::Result<std::fs::File> {
        self.scope.read_file(&self.source_path)
    }
}

#[derive(Deserialize)]
struct BundleManifest {
    manifest: Option<Vec<BundleManifestEntry>>,
}

#[derive(Deserialize)]
struct BundleManifestEntry {
    key: String,
}

pub(crate) fn discover_mod_bundles(
    spt_dir: &Path,
    files: &[crate::db::mods::InstalledFile],
) -> Vec<BundleFile> {
    let mut mod_dirs: std::collections::HashSet<String> = std::collections::HashSet::new();
    for f in files {
        if let Some(rest) = crate::spt::layout::server_mod_suffix(&f.file_path) {
            if let Some(dir) = rest.split('/').next() {
                mod_dirs.insert(dir.to_string());
            }
        }
    }

    let Ok(runtime) = crate::spt::layout::runtime_dir(spt_dir) else {
        return Vec::new();
    };
    let runtime_name = runtime
        .file_name()
        .expect("runtime directory has a name")
        .to_string_lossy();
    let Ok(installation) = crate::spt::scoped::ScopedDir::open(spt_dir) else {
        return Vec::new();
    };
    let mut bundles = Vec::new();
    for mod_dir in &mod_dirs {
        let relative = PathBuf::from(runtime_name.as_ref())
            .join("user/mods")
            .join(mod_dir);
        let manifest_file = match installation.read_file(&relative.join("bundles.json")) {
            Ok(file) => file,
            Err(e) => {
                if !e
                    .downcast_ref::<std::io::Error>()
                    .is_some_and(|e| e.kind() == std::io::ErrorKind::NotFound)
                {
                    tracing::warn!(mod_dir = %mod_dir, err = %e, "failed to read bundle manifest");
                }
                continue;
            }
        };
        let manifest: BundleManifest = match serde_json::from_reader(manifest_file) {
            Ok(m) => m,
            Err(e) => {
                tracing::warn!(mod_dir = %mod_dir, err = %e, "failed to parse bundle manifest");
                continue;
            }
        };
        let entries = manifest.manifest.unwrap_or_default();
        if entries.is_empty() {
            continue;
        }
        let scope = match installation.child(&relative.join("bundles")) {
            Ok(scope) => std::sync::Arc::new(scope),
            Err(e) => {
                tracing::warn!(mod_dir = %mod_dir, err = %e, "unsafe or missing bundle directory, skipping");
                continue;
            }
        };
        for entry in entries {
            let source_path = match crate::spt::scoped::relative_path(&entry.key) {
                Ok(path) => path,
                Err(e) => {
                    tracing::warn!(key = %entry.key, err = %e, "unsafe bundle key, skipping");
                    continue;
                }
            };
            if let Err(e) = scope.read_file(&source_path) {
                tracing::warn!(key = %entry.key, mod_dir = %mod_dir, err = %e, "unsafe or missing bundle file, skipping");
                continue;
            }
            bundles.push(BundleFile {
                cache_path: format!(
                    "{runtime_name}/user/cache/bundles/{}",
                    source_path.to_string_lossy()
                ),
                source_path,
                scope: scope.clone(),
            });
        }
    }

    bundles
}

fn open_client_entry(layers: &LayeredFiles, path: &str) -> anyhow::Result<Option<(PathBuf, File)>> {
    let relative = crate::spt::scoped::relative_path(path)?;
    if crate::spt::layout::is_server_path(&relative.to_string_lossy()) {
        return Ok(None);
    }
    let source = layers.read(&relative, true)?;
    Ok(Some((relative, source)))
}

fn build_convoy_zip(
    dirs: &QumaDirs,
    files: &[InstalledFile],
    bundles: &[BundleFile],
    dest: &mut File,
    bundles_only: bool,
) -> anyhow::Result<()> {
    let mut zip = zip::ZipWriter::new(BufWriter::new(dest));
    let options = zip::write::SimpleFileOptions::default()
        .compression_method(zip::CompressionMethod::Deflated);

    if !bundles_only {
        let layers = LayeredFiles::open(dirs)?;
        for f in files {
            match open_client_entry(&layers, &f.file_path) {
                Ok(Some((relative, mut source))) => {
                    zip.start_file(relative.to_string_lossy().as_ref(), options)?;
                    std::io::copy(&mut source, &mut zip)?;
                }
                Ok(None) => {}
                Err(e) if client_file_checksum(f).is_some() => {
                    return Err(e.context(format!(
                        "cannot include advertised Convoy client file {}",
                        f.file_path
                    )));
                }
                Err(e) => {
                    tracing::warn!(path = %f.file_path, err = %e, "unsafe or missing unadvertised client ZIP source, skipping");
                }
            }
        }
    }

    for b in bundles {
        crate::spt::scoped::relative_path(&b.cache_path)?;
        match b.open() {
            Ok(mut source) => {
                zip.start_file(&b.cache_path, options)?;
                std::io::copy(&mut source, &mut zip)?;
            }
            Err(e) => {
                tracing::warn!(path = %b.cache_path, err = %e, "unsafe or missing bundle file, skipping");
            }
        }
    }

    zip.finish()?.flush()?;
    Ok(())
}
