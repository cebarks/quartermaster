use super::*;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

pub(super) type Sources = BTreeMap<i64, Package>;
pub(crate) type Rebaseline = BTreeMap<i64, (String, i64)>;

#[derive(Clone, Debug, Serialize, Deserialize)]
pub(super) struct Package {
    pub key: String,
    pub guid: Option<String>,
    // Old receipts did not distinguish unused fallback from checked absence.
    #[serde(default)]
    pub fallback_checked: bool,
    pub files: Vec<manifest::AssetEntry>,
    pub bundles: Vec<manifest::AssetEntry>,
}

#[derive(Serialize, Deserialize)]
pub(super) struct Receipt {
    input: String,
    runtime: String,
    sources: Sources,
    manifest: CatalogManifest,
}

pub(super) fn digest(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}

impl CatalogInput {
    pub(super) fn package_key(
        &self,
        m: &InstalledMod,
        runtime: &str,
        spt_version: &str,
    ) -> Result<String> {
        let files = self.files.get(&m.id).map_or(&[][..], Vec::as_slice);
        let mut addons: Vec<_> = self
            .addons
            .iter()
            .filter(|a| a.parent_mod_id == m.id)
            .map(|a| (a.id, a.forge_addon_id, a.forge_version_id, a.disabled))
            .collect();
        addons.sort_unstable();
        Ok(digest(&serde_json::to_vec(&(
            runtime,
            spt_version,
            m.id,
            m.forge_version_id,
            addons,
            file_keys(files),
        ))?))
    }

    fn key(&self, policy: &ConvoyConfig, runtime: &str, spt_version: &str) -> Result<String> {
        let mut mods: Vec<_> = self.mods.iter().collect();
        mods.sort_unstable_by_key(|m| m.id);
        let mut groups: Vec<_> = self
            .groups
            .iter()
            .map(|g| (g.id, &g.slug, &g.name, &g.tier))
            .collect();
        groups.sort_unstable_by_key(|g| g.0);
        let mods = mods
            .into_iter()
            .map(|m| {
                Ok((
                    m.id,
                    m.forge_mod_id,
                    &m.name,
                    &m.guid,
                    &m.version,
                    m.group_id,
                    m.disabled,
                    self.package_key(m, runtime, spt_version)?,
                ))
            })
            .collect::<Result<Vec<_>>>()?;
        Ok(digest(&serde_json::to_vec(&(
            env!("QUMA_VERSION"),
            policy,
            runtime,
            spt_version,
            groups,
            mods,
            file_keys(&self.tracked_files),
        ))?))
    }

    pub(super) fn apply_rebaseline(&mut self, updates: &Rebaseline) {
        for f in self
            .tracked_files
            .iter_mut()
            .chain(self.files.values_mut().flatten())
        {
            if let Some((hash, size)) = updates.get(&f.id) {
                f.file_hash = Some(hash.clone());
                f.file_size = Some(*size);
            }
        }
    }
}

fn file_keys(files: &[InstalledFile]) -> impl Serialize + '_ {
    let mut files: Vec<_> = files
        .iter()
        .map(|f| {
            (
                f.id,
                f.mod_id,
                f.addon_id,
                &f.file_path,
                &f.file_hash,
                f.file_size,
                &f.source,
            )
        })
        .collect();
    files.sort_unstable_by_key(|f| f.0);
    files
}

impl Receipt {
    /// Validate only the stored contents. Database reads and object I/O are
    /// deliberately outside this boundary.
    fn decode(stored: &StoredConvoyReceipt) -> Result<Self> {
        ensure!(
            digest(&stored.data) == stored.id,
            "publication receipt identity mismatch"
        );
        let mut receipt: Self = serde_json::from_slice(&stored.data)?;
        let original = std::mem::take(&mut receipt.manifest.generation);
        receipt.manifest.seal(&receipt.runtime)?;
        ensure!(
            receipt.manifest.generation == original,
            "publication generation mismatch"
        );
        Ok(receipt)
    }

    pub(super) fn committed_digests(
        stored: &StoredConvoyReceipt,
    ) -> Result<impl Iterator<Item = String>> {
        let receipt = Self::decode(stored)?;
        Ok(receipt
            .manifest
            .groups
            .into_iter()
            .flat_map(|g| g.mods)
            .flat_map(|m| m.files.into_iter().chain(m.bundles))
            .map(|asset| asset.sha256))
    }

    pub(super) fn encode(
        input: &CatalogInput,
        policy: &ConvoyConfig,
        runtime: &str,
        snapshot: &PublishedSnapshot,
    ) -> Result<Vec<u8>> {
        Ok(serde_json::to_vec(&Self {
            input: input.key(policy, runtime, &snapshot.manifest.spt_version)?,
            runtime: runtime.to_owned(),
            sources: snapshot.sources.clone(),
            manifest: snapshot.manifest.clone(),
        })?)
    }

    /// A receipt with a stale stamp is not a ready generation. Its individually
    /// matching package inventory can still seed a managed rebuild.
    pub(super) fn restore(
        stored: &StoredConvoyReceipt,
        input: &CatalogInput,
        policy: &ConvoyConfig,
        runtime: &str,
        spt_version: &str,
        store: &ObjectStore,
    ) -> Result<(bool, PublishedSnapshot)> {
        let Self {
            input: identity,
            mut sources,
            manifest,
            ..
        } = Self::decode(stored)?;
        let exact = stored.revision == input.stamp.0
            && identity == input.key(policy, runtime, spt_version)?;
        if !exact {
            let mut matching = BTreeMap::new();
            for m in input.mods.iter().filter(|m| !m.disabled) {
                if let Some(p) = sources.remove(&m.id) {
                    if p.key == input.package_key(m, runtime, spt_version)? {
                        matching.insert(m.id, p);
                    }
                }
            }
            sources = matching;
        }
        let mut objects = BTreeMap::new();
        let mut total = 0u64;
        let assets: Box<dyn Iterator<Item = &manifest::AssetEntry>> = if exact {
            Box::new(
                manifest
                    .groups
                    .iter()
                    .flat_map(|group| &group.mods)
                    .flat_map(|m| m.files.iter().chain(&m.bundles)),
            )
        } else {
            Box::new(
                sources
                    .values()
                    .flat_map(|p| p.files.iter().chain(&p.bundles)),
            )
        };
        for asset in assets {
            if !objects.contains_key(&asset.sha256) {
                let object = store.verify(asset)?;
                total = total
                    .checked_add(object.length)
                    .context("object byte count overflow")?;
                ensure!(
                    total <= policy.max_store_bytes,
                    "publication exceeds object budget"
                );
                objects.insert(asset.sha256.clone(), object);
            }
        }
        let json = serde_json::to_vec(&manifest)?.into();
        Ok((
            exact,
            PublishedSnapshot {
                manifest,
                json,
                objects,
                sources,
            },
        ))
    }
}
