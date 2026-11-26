use crate::{db::mods::InstalledFile, spt::layered::LayeredFiles};
use serde::Deserialize;
#[cfg(test)]
use std::path::PathBuf;
use std::{fs::File, io::Read, path::Path};

#[derive(Deserialize)]
struct BundleManifest {
    manifest: Option<Vec<BundleManifestEntry>>,
}

#[derive(Deserialize)]
pub(super) struct BundleManifestEntry {
    pub(super) key: String,
}

pub(super) fn bundle_mod_dirs(files: &[InstalledFile]) -> impl Iterator<Item = &str> {
    files.iter().filter_map(|f| {
        crate::spt::layout::server_mod_suffix(&f.file_path).and_then(|p| p.split('/').next())
    })
}

#[cfg(test)]
pub(super) fn bundle_manifest(file: File) -> serde_json::Result<Option<Vec<BundleManifestEntry>>> {
    let manifest: BundleManifest = serde_json::from_reader(std::io::BufReader::new(file))?;
    Ok(manifest.manifest)
}

#[allow(dead_code)] // Convoy v2 mounts only in the ephemeral service for now.
fn verified_bundle_manifest(
    file: &mut File,
    tracked: Option<&InstalledFile>,
    rehash: bool,
) -> anyhow::Result<(Option<Vec<BundleManifestEntry>>, String, i64)> {
    use anyhow::ensure;
    // Parse the exact bounded byte buffer verified against the tracked row.
    const MAX_DECLARATION_BYTES: u64 = 8 * 1024 * 1024;
    let mut bytes = Vec::new();
    file.take(MAX_DECLARATION_BYTES + 1)
        .read_to_end(&mut bytes)?;
    ensure!(
        bytes.len() as u64 <= MAX_DECLARATION_BYTES,
        "bundle declaration exceeds metadata limit"
    );
    let actual = format!("{:016x}", xxhash_rust::xxh3::xxh3_64(&bytes));
    if let Some(row) = tracked.filter(|_| !rehash) {
        if let Some(size) = row.file_size {
            ensure!(
                size == bytes.len() as i64,
                "bundle declaration differs from tracked length"
            );
        }
        if let Some(hash) = &row.file_hash {
            ensure!(
                actual == *hash,
                "bundle declaration differs from tracked checksum"
            );
        }
    }
    let declaration: BundleManifest = serde_json::from_slice(&bytes)?;
    Ok((declaration.manifest, actual, bytes.len() as i64))
}

pub(super) fn bundle_cache_path(runtime: &str, key: &str) -> String {
    format!("{runtime}/user/cache/bundles/{key}")
}

/// Strict v2 declaration discovery and payload visitation, separate from the v1
/// ZIP's lenient skips. Open one payload at a time from the live overlay view.
#[allow(dead_code)] // Convoy v2 mounts only in the ephemeral service for now.
pub(super) fn visit_verified_bundle_sources(
    layers: &LayeredFiles,
    runtime: &str,
    files: &[InstalledFile],
    rehash: bool,
    witnesses: &mut super::publisher::Witnesses,
    mut visit: impl FnMut(String, &mut File, Option<&InstalledFile>) -> anyhow::Result<()>,
) -> anyhow::Result<super::publisher::Rebaseline> {
    use crate::spt::layout::server_mod_suffix;
    use anyhow::{ensure, Context};
    let mod_dirs: std::collections::BTreeSet<_> = bundle_mod_dirs(files).collect();
    let tracked: std::collections::BTreeMap<_, _> = files
        .iter()
        .filter_map(|f| server_mod_suffix(&f.file_path).map(|p| (p, f)))
        .collect();
    let mut declarations = super::publisher::Rebaseline::new();
    for mod_dir in mod_dirs {
        ensure!(
            super::manifest::portable_path(mod_dir),
            "invalid bundle mod directory"
        );
        let declaration_suffix = format!("{mod_dir}/bundles.json");
        let declaration_path = format!("{runtime}/user/mods/{declaration_suffix}");
        let declaration = Path::new(&declaration_path);
        let tracked_declaration = tracked.get(declaration_suffix.as_str()).copied();
        let observed = witnesses
            .observe(layers, declaration, false, |source| {
                verified_bundle_manifest(source, tracked_declaration, rehash)
            })
            .with_context(|| format!("invalid bundle declaration for {mod_dir}"))?;
        let Some((entries, hash, length)) = observed else {
            ensure!(
                !tracked.contains_key(declaration_suffix.as_str()),
                "missing tracked bundle declaration for {mod_dir}"
            );
            continue;
        };
        if let Some(row) = tracked_declaration.filter(|_| rehash) {
            declarations.insert(row.id, (hash, length));
        }
        let entries = entries
            .with_context(|| format!("bundle declaration has no manifest list for {mod_dir}"))?;
        let bundle_prefix = format!("{runtime}/user/mods/{mod_dir}/bundles");
        let scope = Path::new(&bundle_prefix);
        for entry in entries {
            ensure!(
                super::manifest::portable_path(&entry.key),
                "invalid bundle key: {:?}",
                entry.key
            );
            let path = scope.join(&entry.key);
            let suffix = format!("{mod_dir}/bundles/{}", entry.key);
            witnesses
                .observe(layers, &path, false, |source| {
                    visit(
                        bundle_cache_path(runtime, &entry.key),
                        source,
                        tracked.get(suffix.as_str()).copied(),
                    )
                })
                .with_context(|| format!("cannot verify declared bundle {}", path.display()))?
                .with_context(|| format!("missing declared bundle {}", path.display()))?;
        }
    }
    Ok(declarations)
}

#[cfg(test)]
pub(crate) struct BundleFile {
    pub(crate) cache_path: String,
    source_path: PathBuf,
    scope: std::sync::Arc<crate::spt::scoped::ScopedDir>,
}

#[cfg(test)]
impl BundleFile {
    pub(crate) fn open(&self) -> anyhow::Result<std::fs::File> {
        self.scope.read_file(&self.source_path)
    }
}

#[cfg(test)]
pub(crate) fn discover_mod_bundles(
    spt_dir: &Path,
    files: &[crate::db::mods::InstalledFile],
) -> Vec<BundleFile> {
    let mod_dirs: std::collections::HashSet<_> = bundle_mod_dirs(files).collect();

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
        let entries = match bundle_manifest(manifest_file) {
            Ok(entries) => entries.unwrap_or_default(),
            Err(e) => {
                tracing::warn!(mod_dir = %mod_dir, err = %e, "failed to parse bundle manifest");
                continue;
            }
        };
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
                cache_path: bundle_cache_path(&runtime_name, &source_path.to_string_lossy()),
                source_path,
                scope: scope.clone(),
            });
        }
    }

    bundles
}
