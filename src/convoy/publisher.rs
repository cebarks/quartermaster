use actix_web::web::Bytes;
use anyhow::{ensure, Context, Result};
use parking_lot::RwLock;
use std::{collections::BTreeMap, fs::File, path::Path, sync::Arc};

use super::{
    bundles::visit_verified_bundle_sources,
    catalog,
    manifest::{self, CatalogGroup, CatalogManifest, CatalogMod},
    object_store::{ObjectRecord, ObjectStore, UnpublishedObjects},
};
use crate::{
    config::{Config, ConvoyConfig},
    db::{
        addons::InstalledAddon,
        convoy::StoredConvoyReceipt,
        mods::{InstalledFile, InstalledMod, ModGroup},
        Database,
    },
    dirs::QumaDirs,
    spt::{layered::LayeredFiles, layout, scoped::ScopedDir},
};

pub struct CatalogInput {
    stamp: (i64, Option<String>),
    committed_receipt: Option<StoredConvoyReceipt>,
    dirs: QumaDirs,
    groups: Vec<ModGroup>,
    mods: Vec<InstalledMod>,
    addons: Vec<InstalledAddon>,
    files: BTreeMap<i64, Vec<InstalledFile>>,
    tracked_files: Vec<InstalledFile>,
    exclusions: Vec<String>,
}

impl CatalogInput {
    /// Captures only database metadata. The caller may release its DB lock
    /// before resolving installation paths or reading any payload.
    pub fn capture(db: &Database, dirs: &QumaDirs, config: &ConvoyConfig) -> Result<Self> {
        Self::capture_metadata(db, dirs, config, false)
    }

    fn capture_metadata(
        db: &Database,
        dirs: &QumaDirs,
        config: &ConvoyConfig,
        recover: bool,
    ) -> Result<Self> {
        Ok(db.read_snapshot(|| {
            let groups = db.list_groups()?;
            let mods = db.list_mods()?;
            let addons = db.list_addons()?;
            let addon_parents: BTreeMap<_, _> = addons
                .iter()
                .filter(|a| !a.disabled)
                .map(|a| (a.id, a.parent_mod_id))
                .collect();
            let tracked_files = db.get_all_tracked_files()?;
            let mut files: BTreeMap<i64, Vec<_>> = BTreeMap::new();
            for f in &tracked_files {
                if let Some(parent) = f
                    .mod_id
                    .or_else(|| f.addon_id.and_then(|id| addon_parents.get(&id).copied()))
                {
                    files.entry(parent).or_default().push(f.clone());
                }
            }
            Ok(Self {
                stamp: db.convoy_stamp()?,
                committed_receipt: if recover { db.convoy_receipt()? } else { None },
                dirs: dirs.clone(),
                groups,
                mods,
                addons,
                files,
                tracked_files,
                exclusions: config.exclusions.clone(),
            })
        })?)
    }
}

#[derive(Debug)]
pub struct PublishedSnapshot {
    pub manifest: CatalogManifest,
    json: Bytes,
    objects: BTreeMap<String, ObjectRecord>,
    sources: receipt::Sources,
}

#[derive(Debug, Clone, Copy)]
pub enum PublishMode {
    Managed,
    Rehash,
}

fn guid(
    layers: &LayeredFiles,
    runtime: &str,
    files: &[InstalledFile],
    witnesses: &mut Witnesses,
) -> Result<Option<String>> {
    let Some(mod_dir) = files
        .iter()
        .find_map(|f| layout::server_mod_suffix(&f.file_path).and_then(|p| p.split('/').next()))
    else {
        return Ok(None);
    };
    if !manifest::portable_path(mod_dir) {
        return Ok(None);
    }
    let path = format!("{runtime}/user/mods/{mod_dir}/package.json");
    Ok(witnesses
        .observe(layers, Path::new(&path), false, |file| {
            Ok(catalog::package_guid(std::io::BufReader::new(file)))
        })?
        .flatten())
}

// Classify before requiring exact canonical namespace spelling, so malformed
// entries cannot disappear from an otherwise eligible inventory.
fn is_candidate_client_file(file: &InstalledFile) -> bool {
    let Some((root, rest)) = file.file_path.split_once('/') else {
        return false;
    };
    if root.eq_ignore_ascii_case("BepInEx") {
        return true;
    }
    file.source == "archive"
        && root.eq_ignore_ascii_case("EscapeFromTarkov_Data")
        && rest.split_once('/').is_some_and(|(dir, name)| {
            dir.eq_ignore_ascii_case("Managed")
                && !name.contains('/')
                && name.len() > 4
                && name.as_bytes()[name.len() - 4..].eq_ignore_ascii_case(b".dll")
        })
}

pub fn build_snapshot(
    input: &CatalogInput,
    store: &ObjectStore,
    previous: Option<&PublishedSnapshot>,
    mode: PublishMode,
) -> Result<PublishedSnapshot> {
    let mut updates = Rebaseline::new();
    let (runtime, version) = installation(input)?;
    let mut witnesses = Witnesses::default();
    let (snapshot, mut owned) = build_candidate(
        input,
        store,
        previous,
        mode,
        &mut updates,
        (runtime, &version),
        &mut witnesses,
    )?;
    witnesses.validate(&input.dirs)?;
    owned.commit();
    Ok(snapshot)
}

pub(super) fn installation(input: &CatalogInput) -> Result<(&'static str, String)> {
    let root = &input.dirs.spt_server;
    Ok((
        layout::detect_layout(root)?.directory(),
        crate::spt::detect::read_spt_version(root)
            .map_or_else(|_| "unknown".into(), |v| v.spt_version),
    ))
}

fn build_candidate(
    input: &CatalogInput,
    store: &ObjectStore,
    previous: Option<&PublishedSnapshot>,
    mode: PublishMode,
    updates: &mut Rebaseline,
    (runtime, spt_version): (&str, &str),
    witnesses: &mut Witnesses,
) -> Result<(PublishedSnapshot, UnpublishedObjects)> {
    let root = &input.dirs.spt_server;
    let scope = ScopedDir::open(root)?;
    let layers = LayeredFiles::open(&input.dirs)?;
    let mut pending = store.candidate()?;
    let mut groups = BTreeMap::new();
    let mut sources = BTreeMap::new();
    let rehash = matches!(mode, PublishMode::Rehash);
    for m in input.mods.iter().filter(|m| !m.disabled) {
        let rows = input.files.get(&m.id).map_or(&[][..], Vec::as_slice);
        let key = input.package_key(m, runtime, spt_version)?;
        let reused = previous.filter(|_| !rehash).and_then(|snapshot| {
            snapshot
                .sources
                .get(&m.id)
                .filter(|p| p.key == key)
                .map(|p| (snapshot, p))
        });
        let mut package = if let Some((snapshot, p)) = reused {
            for asset in p.files.iter().chain(&p.bundles) {
                pending.reuse(
                    asset,
                    snapshot
                        .objects
                        .get(&asset.sha256)
                        .context("missing verified object record")?,
                )?;
            }
            p.clone()
        } else {
            build_package(
                &layers,
                runtime,
                rows,
                key,
                &mut pending,
                (rehash, updates, witnesses),
            )?
        };
        if package.files.is_empty() {
            continue;
        }
        // GUID authority is metadata, independent of the package's byte reuse key.
        if m.guid.is_some() {
            package.guid = None;
            package.fallback_checked = false;
        } else if !package.fallback_checked {
            package.guid = guid(&layers, runtime, rows, witnesses)?;
            package.fallback_checked = true;
        }
        sources.insert(m.id, package.clone());
        let group = match m.group_id {
            None => ("default", "Default", "required"),
            Some(id) => {
                let g = input
                    .groups
                    .iter()
                    .find(|g| g.id == id)
                    .context("Convoy mod references missing group")?;
                (g.slug.as_str(), g.name.as_str(), g.tier.as_str())
            }
        };
        // Key on the database ID: a real group named "default" must not merge
        // with the implicit group. Seal detects the conflicting wire slug.
        groups
            .entry(m.group_id)
            .or_insert_with(|| CatalogGroup {
                slug: group.0.to_owned(),
                name: group.1.to_owned(),
                tier: group.2.to_owned(),
                mods: Vec::new(),
            })
            .mods
            .push(CatalogMod {
                id: m.id,
                forge_id: m.forge_mod_id,
                guid: m.guid.clone().or(package.guid),
                name: m.name.clone(),
                version: m.version.clone(),
                files: package.files,
                bundles: package.bundles,
            });
    }
    let mut groups: Vec<_> = groups.into_values().collect();
    groups.sort_by(|a, b| a.slug.cmp(&b.slug));
    for g in &mut groups {
        g.mods.sort_unstable_by_key(|m| m.id);
    }
    let mut exclusions = input.exclusions.clone();
    exclusions.extend(catalog::scoped_runtime_exclusions(
        &scope,
        &input.tracked_files,
    )?);
    exclusions.sort_unstable();
    exclusions.dedup();
    let mut manifest = CatalogManifest {
        protocol: 2,
        generation: String::new(),
        spt_version: spt_version.to_owned(),
        quartermaster_version: env!("QUMA_VERSION").to_owned(),
        groups,
        exclusions,
    };
    manifest.seal(runtime)?;
    let json = serde_json::to_vec(&manifest)?.into();
    let (objects, ownership) = pending.finish()?;
    Ok((
        PublishedSnapshot {
            manifest,
            json,
            objects,
            sources,
        },
        ownership,
    ))
}

fn build_package(
    layers: &LayeredFiles,
    runtime: &str,
    rows: &[InstalledFile],
    key: String,
    pending: &mut super::object_store::Candidate<'_>,
    (rehash, updates, witnesses): (bool, &mut Rebaseline, &mut Witnesses),
) -> Result<receipt::Package> {
    let mut files = Vec::new();
    for f in rows
        .iter()
        .filter(|f| is_candidate_client_file(f) && !manifest::private_path(&f.file_path))
    {
        let path = &f.file_path;
        ensure!(
            manifest::portable_path(path) && catalog::is_client_path(path),
            "invalid portable client path: {path}"
        );
        let relative = Path::new(path);
        let tracked = if rehash {
            (None, None)
        } else {
            (
                Some(
                    f.file_hash
                        .as_deref()
                        .context("missing tracked client checksum")?,
                ),
                f.file_size,
            )
        };
        let asset = witnesses
            .observe(layers, relative, true, |source| {
                pending.stage(path.clone(), source, tracked)
            })
            .with_context(|| format!("cannot verify Convoy client file {path}"))?
            .with_context(|| format!("missing Convoy client file {path}"))?;
        if rehash {
            updates.insert(f.id, (asset.xxh3_64.clone(), asset.length as i64));
        }
        files.push(asset);
    }
    let mut bundles = Vec::new();
    if !files.is_empty() {
        let declarations = visit_verified_bundle_sources(
            layers,
            runtime,
            rows,
            rehash,
            witnesses,
            |path, source, row| {
                let tracked = row
                    .filter(|_| !rehash)
                    .map_or((None, None), |r| (r.file_hash.as_deref(), r.file_size));
                let asset = pending
                    .stage(path.clone(), source, tracked)
                    .with_context(|| format!("cannot verify declared Convoy bundle {path}"))?;
                if let Some(row) = row.filter(|_| rehash) {
                    updates.insert(row.id, (asset.xxh3_64.clone(), asset.length as i64));
                }
                bundles.push(asset);
                Ok(())
            },
        )?;
        updates.extend(declarations);
    }
    files.sort_unstable_by(|a, b| a.path.cmp(&b.path));
    bundles.sort_unstable_by(|a, b| a.path.cmp(&b.path));
    Ok(receipt::Package {
        key,
        guid: None,
        fallback_checked: false,
        files,
        bundles,
    })
}

pub enum Admission {
    Manifest {
        generation: String,
        json: Bytes,
    },
    Object {
        file: File,
        length: u64,
        lease: ObjectLease,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DeliveryError {
    Disabled,
    Preparing,
    Busy,
    StaleGeneration,
    NotPublished,
    PublishFailed,
}

mod receipt;
mod witness;
pub(crate) use receipt::Rebaseline;
pub(super) use witness::Witnesses;
mod service;
#[cfg(test)]
pub(super) mod test_sync;
#[allow(unused_imports)] // Public status API is mounted by the later production integration.
pub use service::PublicationStatus;
pub use service::{ObjectLease, Publisher};
