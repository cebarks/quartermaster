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
            RawFikaCompat::Bool(false) => Ok(FikaCompat::Incompatible),
            RawFikaCompat::Str(s) => match s.to_lowercase().as_str() {
                "compatible" => Ok(FikaCompat::Compatible),
                "incompatible" => Ok(FikaCompat::Incompatible),
                _ => Ok(FikaCompat::Unknown),
            },
        }
    }
}

/// A mod listing from the Forge API.
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct ForgeMod {
    pub id: i64,
    pub name: String,
    pub slug: Option<String>,
    pub description: Option<String>,
    #[serde(rename = "fika_compatibility")]
    pub fika_compatibility: Option<FikaCompat>,
    pub versions: Option<Vec<ForgeVersion>>,
}

/// A specific version of a mod.
#[derive(Debug, Clone, Deserialize, Serialize, PartialEq)]
pub struct ForgeVersion {
    pub id: i64,
    pub version: String,
    #[serde(alias = "spt_version_constraint")]
    pub spt_version: Option<String>,
    pub link: Option<String>,
    pub content_length: Option<u64>,
    #[serde(rename = "fika_compatibility")]
    pub fika_compatibility: Option<FikaCompat>,
    pub dependencies: Option<Vec<ForgeDependency>>,
}

/// A dependency declared by a mod version.
#[derive(Debug, Clone, PartialEq, Deserialize, Serialize)]
pub struct ForgeDependency {
    pub mod_id: i64,
    pub version_id: Option<i64>,
    pub name: Option<String>,
    pub version: Option<String>,
}

// ---------------------------------------------------------------------------
// Response wrappers
// ---------------------------------------------------------------------------

/// Response envelope for search results.
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct ForgeSearchResponse {
    pub data: Vec<ForgeMod>,
}

/// Response envelope for a single mod.
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct ForgeModResponse {
    pub data: ForgeMod,
}

/// Response envelope for a list of versions.
#[derive(Debug, Clone, Deserialize, Serialize)]
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
    pub name: String,
    pub slug: Option<String>,
    pub latest_compatible_version: Option<ForgeVersion>,
    pub dependencies: Vec<DependencyNode>,
    pub conflict: bool,
}

/// Response envelope for the dependencies endpoint.
#[derive(Debug, Clone, Deserialize, Serialize)]
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
        assert_eq!(m2.fika_compatibility, Some(FikaCompat::Incompatible));
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
}
