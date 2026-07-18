use serde::{Deserialize, Deserializer, Serialize};

/// Fika multiplayer compatibility status.
///
/// The Forge API returns this as a boolean on mod objects (true/false)
/// and as a string enum on version objects ("compatible"/"incompatible"/"unknown").
/// Custom deserialization handles both representations.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum FikaCompat {
    Compatible,
    Incompatible,
    Unknown,
}

/// Internal enum to handle the two different JSON representations via `#[serde(untagged)]`.
#[derive(Deserialize)]
#[serde(untagged)]
enum RawFikaCompat {
    Bool(bool),
    Str(String),
}

impl<'de> Deserialize<'de> for FikaCompat {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let raw = RawFikaCompat::deserialize(deserializer)?;
        match raw {
            RawFikaCompat::Bool(true) => Ok(FikaCompat::Compatible),
            // ponytail: Forge API `false` means "not assessed", not "confirmed incompatible"
            RawFikaCompat::Bool(false) => Ok(FikaCompat::Unknown),
            RawFikaCompat::Str(s) => match s.to_lowercase().as_str() {
                "compatible" => Ok(FikaCompat::Compatible),
                "incompatible" => Ok(FikaCompat::Incompatible),
                _ => Ok(FikaCompat::Unknown),
            },
        }
    }
}

/// A mod or addon owner/author from the Forge API.
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct ForgeModOwner {
    pub id: i64,
    pub name: String,
    pub profile_photo_url: Option<String>,
    pub cover_photo_url: Option<String>,
}

/// A source code link on a mod or addon.
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct SourceCodeLink {
    pub url: String,
    pub label: Option<String>,
}

/// A mod category from the Forge API.
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct ForgeCategory {
    pub id: i64,
    #[serde(alias = "title")]
    pub name: String,
    pub slug: String,
    pub color_class: Option<String>,
}

/// A mod license from the Forge API.
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct ForgeLicense {
    pub id: i64,
    pub name: String,
    pub short_name: String,
}

/// A mod listing from the Forge API.
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct ForgeMod {
    pub id: i64,
    pub hub_id: Option<serde_json::Value>,
    pub guid: Option<String>,
    pub name: String,
    pub slug: Option<String>,
    pub teaser: Option<String>,
    pub description: Option<String>,
    pub thumbnail: Option<String>,
    pub downloads: Option<i64>,
    pub owner: Option<ForgeModOwner>,
    pub additional_authors: Option<Vec<ForgeModOwner>>,
    pub source_code_links: Option<Vec<SourceCodeLink>>,
    pub detail_url: Option<String>,
    pub fika_compatibility: Option<FikaCompat>,
    pub featured: Option<bool>,
    pub contains_ai_content: Option<bool>,
    pub custom_ai_disclosure: Option<String>,
    pub contains_ads: Option<bool>,
    pub shows_profile_binding_notice: Option<bool>,
    pub category: Option<ForgeCategory>,
    pub license: Option<ForgeLicense>,
    pub versions: Option<Vec<ForgeVersion>>,
    pub published_at: Option<String>,
    pub created_at: Option<String>,
    pub updated_at: Option<String>,
}

/// A specific version of a mod.
#[derive(Debug, Clone, Deserialize, Serialize, PartialEq)]
pub struct ForgeVersion {
    pub id: i64,
    pub hub_id: Option<serde_json::Value>,
    pub version: String,
    pub description: Option<String>,
    #[serde(alias = "spt_version_constraint")]
    pub spt_version: Option<String>,
    pub link: Option<String>,
    pub content_length: Option<u64>,
    pub downloads: Option<i64>,
    pub fika_compatibility: Option<FikaCompat>,
    pub dependencies: Option<Vec<ForgeDependency>>,
    pub published_at: Option<String>,
    pub created_at: Option<String>,
    pub updated_at: Option<String>,
}

/// A dependency declared by a mod version.
#[derive(Debug, Clone, PartialEq, Deserialize, Serialize)]
pub struct ForgeDependency {
    pub mod_id: i64,
    pub mod_guid: Option<String>,
    pub version_id: Option<i64>,
    #[serde(alias = "mod_name")]
    pub name: Option<String>,
    #[serde(alias = "version_constraint")]
    pub version: Option<String>,
    pub is_optional: Option<bool>,
}

// ---------------------------------------------------------------------------
// Pagination types
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Deserialize)]
pub(crate) struct PaginationLinks {
    pub next: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
pub(crate) struct PaginatedResponse<T> {
    pub data: T,
    pub links: Option<PaginationLinks>,
}

// ---------------------------------------------------------------------------
// Response wrappers
// ---------------------------------------------------------------------------

/// Response envelope for search results.
#[derive(Debug, Clone, Deserialize, Serialize)]
#[allow(dead_code)] // superseded by DataWrapper<T> in client.rs
pub struct ForgeSearchResponse {
    pub data: Vec<ForgeMod>,
}

/// Response envelope for a single mod.
#[derive(Debug, Clone, Deserialize, Serialize)]
#[allow(dead_code)] // superseded by DataWrapper<T> in client.rs
pub struct ForgeModResponse {
    pub data: ForgeMod,
}

/// Response envelope for a list of versions.
#[derive(Debug, Clone, Deserialize, Serialize)]
#[allow(dead_code)] // superseded by DataWrapper<T> in client.rs
pub struct ForgeVersionsResponse {
    pub data: Vec<ForgeVersion>,
}

// ---------------------------------------------------------------------------
// Dependency resolution types
// ---------------------------------------------------------------------------

/// A node in the resolved dependency tree.
#[derive(Debug, Clone, Deserialize, Serialize, PartialEq)]
pub struct DependencyNode {
    pub id: i64,
    pub guid: Option<String>,
    pub name: String,
    pub slug: Option<String>,
    pub latest_compatible_version: Option<ForgeVersion>,
    pub dependencies: Vec<DependencyNode>,
    pub conflict: bool,
}

/// Response envelope for the dependencies endpoint.
#[derive(Debug, Clone, Deserialize, Serialize)]
#[allow(dead_code)] // superseded by DataWrapper<T> in client.rs
pub struct DependencyResponse {
    pub data: Vec<DependencyNode>,
}

// ---------------------------------------------------------------------------
// Update check types
// ---------------------------------------------------------------------------

/// A mod entry in the update check response.
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct UpdateCheckMod {
    pub id: i64,
    pub mod_id: i64,
    pub guid: Option<String>,
    pub name: String,
    pub slug: Option<String>,
    pub version: String,
}

/// Recommended version information for an update.
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct UpdateRecommendedVersion {
    pub id: i64,
    pub version: String,
    pub link: Option<String>,
    pub content_length: Option<u64>,
    pub spt_versions: Option<Vec<String>>,
    pub fika_compatibility: Option<FikaCompat>,
}

/// A single update entry.
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct UpdateEntry {
    pub current_version: UpdateCheckMod,
    pub recommended_version: UpdateRecommendedVersion,
    pub update_reason: String,
}

/// A mod incompatible with the given SPT version.
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct IncompatibleMod {
    pub id: i64,
    pub mod_id: i64,
    pub name: String,
    pub version: String,
}

/// The data portion of the updates response.
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct UpdatesResponseData {
    pub spt_version: String,
    pub updates: Vec<UpdateEntry>,
    pub blocked_updates: Vec<serde_json::Value>,
    pub up_to_date: Vec<serde_json::Value>,
    pub incompatible_with_spt: Vec<IncompatibleMod>,
}

/// Response envelope for the updates endpoint.
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct UpdatesResponse {
    pub data: UpdatesResponseData,
}

// ---------------------------------------------------------------------------
// Addon types
// ---------------------------------------------------------------------------

/// An addon listing from the Forge API.
#[derive(Debug, Clone, Deserialize, Serialize)]
#[allow(dead_code)] // used in Task 4
pub struct ForgeAddon {
    pub id: i64,
    pub name: String,
    pub slug: Option<String>,
    pub teaser: Option<String>,
    pub description: Option<String>,
    pub thumbnail: Option<String>,
    pub downloads: Option<i64>,
    pub owner: Option<ForgeModOwner>,
    pub additional_authors: Option<Vec<ForgeModOwner>>,
    pub source_code_links: Option<Vec<SourceCodeLink>>,
    pub detail_url: Option<String>,
    pub contains_ai_content: Option<bool>,
    pub custom_ai_disclosure: Option<String>,
    pub contains_ads: Option<bool>,
    pub mod_id: Option<i64>,
    pub is_detached: Option<bool>,
    pub versions: Option<Vec<ForgeAddonVersion>>,
    pub published_at: Option<String>,
    pub created_at: Option<String>,
    pub updated_at: Option<String>,
}

/// A specific version of an addon.
#[derive(Debug, Clone, Deserialize, Serialize, PartialEq)]
#[allow(dead_code)] // used in Task 4
pub struct ForgeAddonVersion {
    pub id: i64,
    pub version: String,
    pub description: Option<String>,
    pub link: Option<String>,
    pub content_length: Option<u64>,
    pub mod_version_constraint: Option<String>,
    pub downloads: Option<i64>,
    pub published_at: Option<String>,
    pub created_at: Option<String>,
    pub updated_at: Option<String>,
}

/// Response envelope for addon search results.
#[derive(Debug, Clone, Deserialize, Serialize)]
#[allow(dead_code)] // used in Task 5
pub struct ForgeAddonSearchResponse {
    pub data: Vec<ForgeAddon>,
}

/// Response envelope for a single addon.
#[derive(Debug, Clone, Deserialize, Serialize)]
#[allow(dead_code)] // used in Task 5
pub struct ForgeAddonResponse {
    pub data: ForgeAddon,
}

/// Response envelope for a list of addon versions.
#[derive(Debug, Clone, Deserialize, Serialize)]
#[allow(dead_code)] // used in Task 5
pub struct ForgeAddonVersionsResponse {
    pub data: Vec<ForgeAddonVersion>,
}

// ---------------------------------------------------------------------------
// SPT version types
// ---------------------------------------------------------------------------

/// An SPT version from the Forge API.
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct SptVersion {
    pub id: i64,
    pub version: String,
    pub version_major: Option<i64>,
    pub version_minor: Option<i64>,
    pub version_patch: Option<i64>,
    pub version_labels: Option<String>,
    pub mod_count: Option<i64>,
    pub link: Option<String>,
    pub color_class: Option<String>,
    pub created_at: Option<String>,
    pub updated_at: Option<String>,
}

/// Response envelope for SPT versions.
#[derive(Debug, Clone, Deserialize, Serialize)]
#[allow(dead_code)] // superseded by DataWrapper<T> in client.rs
pub struct SptVersionsResponse {
    pub data: Vec<SptVersion>,
}

// ---------------------------------------------------------------------------
// Version comparison helpers
// ---------------------------------------------------------------------------

/// Parse a version string into a comparable tuple of numeric parts.
/// Returns None if the string contains no parseable numeric segments.
fn parse_version_parts(version: &str) -> Option<Vec<u64>> {
    let parts: Vec<u64> = version
        .split('.')
        .filter_map(|s| {
            // Take leading digits from each segment (handles "1.2.3-beta" → [1, 2, 3])
            let digits: String = s.chars().take_while(|c| c.is_ascii_digit()).collect();
            digits.parse().ok()
        })
        .collect();
    if parts.is_empty() {
        None
    } else {
        Some(parts)
    }
}

/// Return the latest version by semver ordering.
/// Falls back to highest Forge version ID for unparseable version strings.
pub fn latest_version(versions: &[ForgeVersion]) -> Option<&ForgeVersion> {
    versions.iter().max_by(|a, b| {
        let pa = parse_version_parts(&a.version);
        let pb = parse_version_parts(&b.version);
        match (pa, pb) {
            (Some(ref va), Some(ref vb)) => va.cmp(vb).then(a.id.cmp(&b.id)),
            (Some(_), None) => std::cmp::Ordering::Greater,
            (None, Some(_)) => std::cmp::Ordering::Less,
            (None, None) => a.id.cmp(&b.id),
        }
    })
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;

    #[test]
    fn deserialize_forge_mod() {
        let json = r#"{
            "id": 42,
            "name": "Big Brain",
            "slug": "big-brain",
            "description": "AI overhaul mod",
            "fika_compatibility": true,
            "versions": [
                {
                    "id": 100,
                    "version": "1.2.0",
                    "spt_version": "3.9.0",
                    "link": "https://example.com/download",
                    "content_length": 1048576,
                    "fika_compatibility": "compatible",
                    "dependencies": []
                }
            ]
        }"#;

        let m: ForgeMod = serde_json::from_str(json).expect("should deserialize ForgeMod");
        assert_eq!(m.id, 42);
        assert_eq!(m.name, "Big Brain");
        assert_eq!(m.slug.as_deref(), Some("big-brain"));
        assert_eq!(m.description.as_deref(), Some("AI overhaul mod"));
        assert_eq!(m.fika_compatibility, Some(FikaCompat::Compatible));
        let versions = m.versions.expect("should have versions");
        assert_eq!(versions.len(), 1);
        assert_eq!(versions[0].id, 100);
        assert_eq!(versions[0].version, "1.2.0");
        assert_eq!(versions[0].fika_compatibility, Some(FikaCompat::Compatible));
    }

    #[test]
    fn fika_compat_from_bool() {
        let json_true = r#"{"id":1,"name":"a","fika_compatibility":true}"#;
        let json_false = r#"{"id":2,"name":"b","fika_compatibility":false}"#;

        let m1: ForgeMod = serde_json::from_str(json_true).unwrap();
        let m2: ForgeMod = serde_json::from_str(json_false).unwrap();

        assert_eq!(m1.fika_compatibility, Some(FikaCompat::Compatible));
        assert_eq!(m2.fika_compatibility, Some(FikaCompat::Unknown));
    }

    #[test]
    fn fika_compat_from_string() {
        let json = r#"{
            "id": 10,
            "version": "2.0.0",
            "fika_compatibility": "compatible"
        }"#;
        let v: ForgeVersion = serde_json::from_str(json).unwrap();
        assert_eq!(v.fika_compatibility, Some(FikaCompat::Compatible));
    }

    #[test]
    fn fika_compat_unknown_string() {
        let json = r#"{
            "id": 11,
            "version": "2.0.0",
            "fika_compatibility": "unknown"
        }"#;
        let v: ForgeVersion = serde_json::from_str(json).unwrap();
        assert_eq!(v.fika_compatibility, Some(FikaCompat::Unknown));
    }

    #[test]
    fn fika_compat_serialization_round_trips() {
        // Serialize each variant and verify lowercase output
        let compat = FikaCompat::Compatible;
        let json = serde_json::to_string(&compat).unwrap();
        assert_eq!(json, r#""compatible""#);

        let incompat = FikaCompat::Incompatible;
        let json = serde_json::to_string(&incompat).unwrap();
        assert_eq!(json, r#""incompatible""#);

        let unknown = FikaCompat::Unknown;
        let json = serde_json::to_string(&unknown).unwrap();
        assert_eq!(json, r#""unknown""#);

        // Round-trip: serialize then deserialize back
        for variant in [
            FikaCompat::Compatible,
            FikaCompat::Incompatible,
            FikaCompat::Unknown,
        ] {
            let serialized = serde_json::to_string(&variant).unwrap();
            let deserialized: FikaCompat = serde_json::from_str(&serialized).unwrap();
            assert_eq!(variant, deserialized);
        }
    }

    #[test]
    fn fika_compat_missing() {
        let json = r#"{"id":3,"name":"c"}"#;
        let m: ForgeMod = serde_json::from_str(json).unwrap();
        assert_eq!(m.fika_compatibility, None);
    }

    #[test]
    fn deserialize_version_with_all_fields() {
        let json = r#"{
            "id": 200,
            "version": "3.1.0",
            "spt_version": "3.10.0",
            "link": "https://forge.sp-tarkov.com/files/200",
            "content_length": 2097152,
            "fika_compatibility": "incompatible",
            "dependencies": [
                {
                    "mod_id": 50,
                    "version_id": 150,
                    "name": "CoreLib",
                    "version": "1.0.0"
                },
                {
                    "mod_id": 51,
                    "version_id": null,
                    "name": "OptionalLib",
                    "version": null
                }
            ]
        }"#;

        let v: ForgeVersion = serde_json::from_str(json).unwrap();
        assert_eq!(v.id, 200);
        assert_eq!(v.version, "3.1.0");
        assert_eq!(v.spt_version.as_deref(), Some("3.10.0"));
        assert_eq!(
            v.link.as_deref(),
            Some("https://forge.sp-tarkov.com/files/200")
        );
        assert_eq!(v.content_length, Some(2_097_152));
        assert_eq!(v.fika_compatibility, Some(FikaCompat::Incompatible));

        let deps = v.dependencies.expect("should have dependencies");
        assert_eq!(deps.len(), 2);
        assert_eq!(deps[0].mod_id, 50);
        assert_eq!(deps[0].version_id, Some(150));
        assert_eq!(deps[0].name.as_deref(), Some("CoreLib"));
        assert_eq!(deps[1].version_id, None);
        assert_eq!(deps[1].version.as_deref(), None);
    }

    #[test]
    fn deserialize_dependency_with_api_field_names() {
        let json = r#"{
            "mod_id": 42,
            "mod_guid": "com.example.core-library",
            "mod_name": "Core Library",
            "version_constraint": "^2.0.0",
            "is_optional": false
        }"#;

        let dep: ForgeDependency = serde_json::from_str(json).unwrap();
        assert_eq!(dep.mod_id, 42);
        assert_eq!(dep.mod_guid.as_deref(), Some("com.example.core-library"));
        assert_eq!(dep.name.as_deref(), Some("Core Library"));
        assert_eq!(dep.version.as_deref(), Some("^2.0.0"));
        assert_eq!(dep.is_optional, Some(false));
        assert_eq!(dep.version_id, None);
    }

    #[test]
    fn deserialize_abbreviated_version() {
        let json = r#"{
            "id": 300,
            "version": "0.1.0"
        }"#;

        let v: ForgeVersion = serde_json::from_str(json).unwrap();
        assert_eq!(v.id, 300);
        assert_eq!(v.version, "0.1.0");
        assert_eq!(v.spt_version, None);
        assert_eq!(v.link, None);
        assert_eq!(v.content_length, None);
        assert_eq!(v.fika_compatibility, None);
        assert_eq!(v.dependencies, None);
    }

    #[test]
    fn deserialize_dependency_node() {
        let json = r#"{
            "id": 902,
            "name": "BigBrain",
            "slug": "bigbrain",
            "latest_compatible_version": {
                "id": 11761,
                "version": "1.4.0",
                "link": "https://github.com/.../BigBrain-1.4.0.7z",
                "content_length": 16741,
                "spt_version_constraint": "~4.0.0",
                "fika_compatibility": "unknown"
            },
            "dependencies": [
                {
                    "id": 123,
                    "name": "ChildMod",
                    "slug": "child-mod",
                    "latest_compatible_version": {
                        "id": 456,
                        "version": "0.5.0",
                        "link": "https://example.com/child.7z",
                        "content_length": 1024,
                        "spt_version_constraint": "~4.0.0",
                        "fika_compatibility": "compatible"
                    },
                    "dependencies": [],
                    "conflict": false
                }
            ],
            "conflict": false
        }"#;

        let node: DependencyNode = serde_json::from_str(json).unwrap();
        assert_eq!(node.id, 902);
        assert_eq!(node.name, "BigBrain");
        assert_eq!(node.slug.as_deref(), Some("bigbrain"));
        assert_eq!(node.conflict, false);

        let version = node
            .latest_compatible_version
            .expect("should have latest_compatible_version");
        assert_eq!(version.id, 11761);
        assert_eq!(version.version, "1.4.0");
        assert_eq!(
            version.spt_version.as_deref(),
            Some("~4.0.0"),
            "spt_version_constraint should deserialize via alias"
        );

        assert_eq!(node.dependencies.len(), 1);
        assert_eq!(node.dependencies[0].id, 123);
        assert_eq!(node.dependencies[0].name, "ChildMod");
        assert!(node.dependencies[0].dependencies.is_empty());
    }

    #[test]
    fn deserialize_forge_mod_all_fields() {
        let json = r#"{
            "id": 42,
            "hub_id": "123",
            "guid": "com.example.big-brain",
            "name": "Big Brain",
            "slug": "big-brain",
            "teaser": "AI overhaul mod for SPT",
            "description": "Full description here",
            "thumbnail": "https://forge.sp-tarkov.com/thumbs/42.jpg",
            "downloads": 55212644,
            "owner": {
                "id": 1,
                "name": "ModAuthor",
                "profile_photo_url": "https://example.com/profile.jpg",
                "cover_photo_url": "https://example.com/cover.jpg"
            },
            "additional_authors": [
                {
                    "id": 2,
                    "name": "CoAuthor",
                    "profile_photo_url": "https://example.com/co.jpg",
                    "cover_photo_url": null
                }
            ],
            "source_code_links": [
                {"url": "https://github.com/example/big-brain", "label": "GitHub"}
            ],
            "detail_url": "https://forge.sp-tarkov.com/mod/42/big-brain",
            "fika_compatibility": true,
            "featured": true,
            "contains_ai_content": false,
            "custom_ai_disclosure": null,
            "contains_ads": false,
            "shows_profile_binding_notice": false,
            "published_at": "2025-01-09T17:48:53.000000Z",
            "created_at": "2024-12-11T14:48:53.000000Z",
            "updated_at": "2025-04-10T13:50:00.000000Z"
        }"#;

        let m: ForgeMod = serde_json::from_str(json).expect("should deserialize full ForgeMod");
        assert_eq!(m.id, 42);
        assert_eq!(m.guid.as_deref(), Some("com.example.big-brain"));
        assert_eq!(m.teaser.as_deref(), Some("AI overhaul mod for SPT"));
        assert_eq!(m.downloads, Some(55212644));
        assert_eq!(
            m.detail_url.as_deref(),
            Some("https://forge.sp-tarkov.com/mod/42/big-brain")
        );
        assert_eq!(m.featured, Some(true));
        assert_eq!(m.contains_ai_content, Some(false));
        assert_eq!(m.contains_ads, Some(false));
        assert_eq!(m.fika_compatibility, Some(FikaCompat::Compatible));

        let owner = m.owner.expect("should have owner");
        assert_eq!(owner.id, 1);
        assert_eq!(owner.name, "ModAuthor");

        let authors = m
            .additional_authors
            .expect("should have additional_authors");
        assert_eq!(authors.len(), 1);
        assert_eq!(authors[0].name, "CoAuthor");

        let links = m.source_code_links.expect("should have source_code_links");
        assert_eq!(links.len(), 1);
        assert_eq!(links[0].url, "https://github.com/example/big-brain");
        assert_eq!(links[0].label.as_deref(), Some("GitHub"));

        assert!(m.published_at.is_some());
        assert!(m.created_at.is_some());
        assert!(m.updated_at.is_some());
    }

    #[test]
    fn deserialize_forge_version_all_fields() {
        let json = r#"{
            "id": 200,
            "hub_id": "456",
            "version": "3.1.0",
            "description": "Major update with new features",
            "spt_version_constraint": "^3.10.0",
            "link": "https://forge.sp-tarkov.com/files/200",
            "content_length": 2097152,
            "downloads": 1523,
            "fika_compatibility": "compatible",
            "dependencies": [],
            "published_at": "2025-01-09T17:48:53.000000Z",
            "created_at": "2024-12-11T14:48:53.000000Z",
            "updated_at": "2025-04-10T13:50:00.000000Z"
        }"#;

        let v: ForgeVersion =
            serde_json::from_str(json).expect("should deserialize full ForgeVersion");
        assert_eq!(v.id, 200);
        assert_eq!(
            v.description.as_deref(),
            Some("Major update with new features")
        );
        assert_eq!(v.downloads, Some(1523));
        assert!(v.published_at.is_some());
        assert!(v.created_at.is_some());
        assert!(v.updated_at.is_some());
    }

    #[test]
    fn deserialize_forge_mod_with_category_and_license() {
        let json = r#"{
            "id": 42,
            "name": "Big Brain",
            "fika_compatibility": true,
            "category": {
                "id": 1,
                "name": "Gameplay",
                "slug": "gameplay",
                "color_class": "blue"
            },
            "license": {
                "id": 1,
                "name": "MIT License",
                "short_name": "MIT"
            }
        }"#;

        let m: ForgeMod = serde_json::from_str(json).expect("should deserialize with includes");
        let cat = m.category.expect("should have category");
        assert_eq!(cat.id, 1);
        assert_eq!(cat.name, "Gameplay");
        assert_eq!(cat.slug, "gameplay");
        assert_eq!(cat.color_class.as_deref(), Some("blue"));

        let lic = m.license.expect("should have license");
        assert_eq!(lic.id, 1);
        assert_eq!(lic.name, "MIT License");
        assert_eq!(lic.short_name, "MIT");
    }

    #[test]
    fn deserialize_category_with_title_field() {
        let json = r#"{
            "id": 42,
            "name": "NarcoNet",
            "category": {
                "id": 1,
                "hub_id": 17,
                "title": "Tools",
                "slug": "tools",
                "description": "Various standalone tools"
            }
        }"#;

        let m: ForgeMod = serde_json::from_str(json).expect("category with 'title' should work");
        let cat = m.category.expect("should have category");
        assert_eq!(cat.name, "Tools");
        assert_eq!(cat.slug, "tools");
    }

    #[test]
    fn deserialize_dependency_node_with_guid() {
        let json = r#"{
            "id": 42,
            "guid": "com.example.big-brain",
            "name": "Big Brain",
            "slug": "big-brain",
            "latest_compatible_version": {
                "id": 100,
                "version": "1.2.0"
            },
            "dependencies": [],
            "conflict": false
        }"#;

        let node: DependencyNode =
            serde_json::from_str(json).expect("should deserialize with guid");
        assert_eq!(node.id, 42);
        assert_eq!(node.guid.as_deref(), Some("com.example.big-brain"));
    }

    #[test]
    fn deserialize_forge_addon() {
        let json = r#"{
            "id": 1,
            "name": "Ultimate Music Pack",
            "slug": "ultimate-music-pack",
            "teaser": "A collection of atmospheric music tracks",
            "description": "This addon adds over 50 new music tracks...",
            "thumbnail": "",
            "downloads": 1523,
            "owner": {
                "id": 1,
                "name": "AddonAuthor",
                "profile_photo_url": "https://example.com/profile.jpg",
                "cover_photo_url": null
            },
            "additional_authors": [],
            "source_code_links": [],
            "detail_url": "https://forge.sp-tarkov.com/addon/1/ultimate-music-pack",
            "contains_ads": false,
            "contains_ai_content": false,
            "mod_id": 5,
            "is_detached": false,
            "published_at": "2025-01-09T17:48:53.000000Z",
            "created_at": "2024-12-11T14:48:53.000000Z",
            "updated_at": "2025-04-10T13:50:00.000000Z"
        }"#;

        let a: ForgeAddon = serde_json::from_str(json).expect("should deserialize ForgeAddon");
        assert_eq!(a.id, 1);
        assert_eq!(a.name, "Ultimate Music Pack");
        assert_eq!(a.mod_id, Some(5));
        assert_eq!(a.is_detached, Some(false));
        assert_eq!(a.downloads, Some(1523));
    }

    #[test]
    fn deserialize_forge_addon_version() {
        let json = r#"{
            "id": 1,
            "version": "1.2.0",
            "description": "Added 10 new tracks",
            "link": "https://example.com/download/v1.2.0.zip",
            "content_length": 52428800,
            "mod_version_constraint": "^2.0.0",
            "downloads": 523,
            "published_at": "2025-01-09T17:48:53.000000Z",
            "created_at": "2024-12-11T14:48:53.000000Z",
            "updated_at": "2025-04-10T13:50:00.000000Z"
        }"#;

        let v: ForgeAddonVersion =
            serde_json::from_str(json).expect("should deserialize ForgeAddonVersion");
        assert_eq!(v.id, 1);
        assert_eq!(v.version, "1.2.0");
        assert_eq!(v.mod_version_constraint.as_deref(), Some("^2.0.0"));
        assert_eq!(v.content_length, Some(52428800));
        assert_eq!(v.downloads, Some(523));
    }

    #[test]
    fn deserialize_spt_version() {
        let json = r#"{
            "id": 2,
            "version": "3.11.3",
            "version_major": 3,
            "version_minor": 11,
            "version_patch": 3,
            "version_labels": "",
            "mod_count": 371,
            "link": "https://github.com/sp-tarkov/build/releases/tag/3.11.3",
            "color_class": "green",
            "created_at": "2025-04-08T19:29:40.000000Z",
            "updated_at": "2025-04-08T19:29:40.000000Z"
        }"#;

        let v: SptVersion = serde_json::from_str(json).expect("should deserialize SptVersion");
        assert_eq!(v.id, 2);
        assert_eq!(v.version, "3.11.3");
        assert_eq!(v.version_major, Some(3));
        assert_eq!(v.version_minor, Some(11));
        assert_eq!(v.version_patch, Some(3));
        assert_eq!(v.mod_count, Some(371));
        assert_eq!(
            v.link.as_deref(),
            Some("https://github.com/sp-tarkov/build/releases/tag/3.11.3")
        );
        assert_eq!(v.color_class.as_deref(), Some("green"));
    }

    // Helper for latest_version tests
    fn ver(id: i64, version: &str) -> ForgeVersion {
        ForgeVersion {
            id,
            hub_id: None,
            version: version.to_string(),
            description: None,
            spt_version: None,
            link: None,
            content_length: None,
            downloads: None,
            fika_compatibility: None,
            dependencies: None,
            published_at: None,
            created_at: None,
            updated_at: None,
        }
    }

    #[test]
    fn latest_version_semver_ordering() {
        let versions = vec![ver(1, "1.0.0"), ver(2, "2.0.0"), ver(3, "1.5.0")];
        assert_eq!(latest_version(&versions).unwrap().id, 2);
    }

    #[test]
    fn latest_version_falls_back_to_forge_id() {
        let versions = vec![ver(10, "alpha"), ver(20, "beta")];
        assert_eq!(latest_version(&versions).unwrap().id, 20);
    }

    #[test]
    fn latest_version_mixed_parseable_and_not() {
        let versions = vec![ver(1, "1.0.0"), ver(100, "beta-rc1")];
        // "1.0.0" parses to (1,0,0); "beta-rc1" doesn't parse, gets (0,0,0) sentinel
        // So "1.0.0" wins on semver
        assert_eq!(latest_version(&versions).unwrap().id, 1);
    }

    #[test]
    fn latest_version_empty() {
        let versions: Vec<ForgeVersion> = vec![];
        assert!(latest_version(&versions).is_none());
    }

    #[test]
    fn latest_version_four_part() {
        let versions = vec![ver(1, "1.2.3.4"), ver(2, "1.2.3.5")];
        assert_eq!(latest_version(&versions).unwrap().id, 2);
    }
}
