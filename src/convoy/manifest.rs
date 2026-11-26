use anyhow::{ensure, Result};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};
use xxhash_rust::xxh3::xxh3_128;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CatalogManifest {
    pub protocol: u8,
    #[serde(skip_serializing_if = "String::is_empty")]
    pub generation: String,
    pub spt_version: String,
    pub quartermaster_version: String,
    pub groups: Vec<CatalogGroup>,
    pub exclusions: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CatalogGroup {
    pub slug: String,
    pub name: String,
    pub tier: String,
    pub mods: Vec<CatalogMod>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CatalogMod {
    pub id: i64,
    pub forge_id: Option<i64>,
    pub guid: Option<String>,
    pub name: String,
    pub version: String,
    pub files: Vec<AssetEntry>,
    pub bundles: Vec<AssetEntry>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AssetEntry {
    pub path: String,
    pub length: u64,
    pub xxh3_128: String,
}

pub(super) fn lower_hex(s: &str, length: usize) -> bool {
    s.len() == length
        && s.bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
}

pub(super) fn private_path(path: &str) -> bool {
    let mut components = path.split('/');
    ["BepInEx", "config", "Convoy"].iter().all(|part| {
        components
            .next()
            .is_some_and(|name| name.eq_ignore_ascii_case(part))
    })
}

pub(super) fn portable_path(path: &str) -> bool {
    super::catalog::portable_path(path)
        && !private_path(path)
        && !path.split('/').any(|p| {
            matches!(
                p.split('.')
                    .next()
                    .unwrap_or_default()
                    .to_ascii_uppercase()
                    .as_str(),
                "COM¹" | "COM²" | "COM³" | "LPT¹" | "LPT²" | "LPT³" | "CONIN$" | "CONOUT$"
            )
        })
}

impl CatalogManifest {
    pub(super) fn seal(&mut self, runtime: &str) -> Result<()> {
        ensure!(self.protocol == 3, "unsupported Convoy protocol");
        let mut groups = BTreeSet::new();
        let mut mods = BTreeSet::new();
        let mut paths = BTreeMap::new();
        let mut digests = BTreeMap::new();
        let bundle_prefix = format!("{runtime}/user/cache/bundles/");
        for group in &self.groups {
            ensure!(
                !group.slug.is_empty() && groups.insert(&group.slug),
                "duplicate or empty Convoy group slug"
            );
            ensure!(
                matches!(group.tier.as_str(), "required" | "optional"),
                "invalid Convoy group tier"
            );
            for package in &group.mods {
                ensure!(
                    package.id > 0 && mods.insert(package.id),
                    "invalid or duplicate Convoy mod id"
                );
                let mut seen = BTreeSet::new();
                for (entries, bundles) in [(&package.files, false), (&package.bundles, true)] {
                    for entry in entries {
                        ensure!(
                            portable_path(&entry.path)
                                && !entry.path.eq_ignore_ascii_case("BepInEx/config")
                                && if bundles {
                                    entry.path.starts_with(&bundle_prefix)
                                } else {
                                    super::catalog::is_client_path(&entry.path)
                                },
                            "unsafe Convoy path {:?}",
                            entry.path
                        );
                        ensure!(
                            seen.insert(&entry.path),
                            "duplicate Convoy mod path: {}",
                            entry.path
                        );
                        ensure!(
                            entry.length <= i64::MAX as u64 && lower_hex(&entry.xxh3_128, 32),
                            "invalid Convoy object identity"
                        );
                        if let Some(length) = digests.insert(&entry.xxh3_128, entry.length) {
                            ensure!(length == entry.length, "conflicting Convoy object lengths");
                        }
                        // Insert all parents as directory nodes as well; reject aliases and
                        // parent/file collisions regardless of group selection or order.
                        for (part, identity) in entry
                            .path
                            .match_indices('/')
                            .map(|(i, _)| (&entry.path[..i], None))
                            .chain(std::iter::once((entry.path.as_str(), Some(entry))))
                        {
                            let value = (part, identity);
                            if let Some(old) = paths.insert(part.to_lowercase(), value) {
                                ensure!(old == value, "Convoy path alias or collision: {part}");
                            }
                        }
                    }
                }
            }
        }
        self.generation = format!("{:032x}", xxh3_128(&serde_json::to_vec(self)?));
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn repeated_digest_requires_one_length_across_optional_groups_and_bundles() {
        let mut manifest: CatalogManifest = serde_json::from_value(serde_json::json!({
            "protocol": 3, "generation": "", "spt_version": "test", "quartermaster_version": "test",
            "groups": [
                {"slug": "required", "name": "Required", "tier": "required", "mods": [{
                    "id": 1, "forge_id": null, "guid": null, "name": "A", "version": "1",
                    "files": [{"path": "BepInEx/plugins/a.dll", "length": 3,
                        "xxh3_128": "06b05ab6733a618578af5f94892f3950"}], "bundles": []}]},
                {"slug": "optional", "name": "Optional", "tier": "optional", "mods": [{
                    "id": 2, "forge_id": null, "guid": null, "name": "B", "version": "1",
                    "files": [], "bundles": [{"path": "SPT_Runtime/user/cache/bundles/shared.bundle",
                        "length": 3, "xxh3_128": "06b05ab6733a618578af5f94892f3950"}]}]}
            ], "exclusions": []
        })).unwrap();
        assert!(
            manifest.seal("SPT_Runtime").is_ok(),
            "matching shared identity is allowed"
        );
        manifest.groups[1].mods[0].bundles[0].length = 4;
        assert!(
            manifest.seal("SPT_Runtime").is_err(),
            "conflicting lengths must be rejected before selection"
        );
    }
}
