use serde::{Deserialize, Deserializer, Serialize};

/// Fika multiplayer compatibility status.
///
/// The Forge API returns this as a boolean on mod objects (true/false)
/// and as a string enum on version objects ("compatible"/"incompatible"/"unknown").
/// Custom deserialization handles both representations.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
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
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct ForgeVersion {
    pub id: i64,
    pub version: String,
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
    pub mod_id: i64,
    pub version_id: i64,
    pub name: Option<String>,
    pub version: Option<String>,
    pub resolved_dependencies: Option<Vec<DependencyNode>>,
}

/// Result of checking a single mod for updates.
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct UpdateCheckResult {
    pub mod_id: i64,
    pub current_version: String,
    pub latest_version: Option<String>,
    pub latest_version_id: Option<i64>,
    pub status: String,
}

/// Response envelope for update checks.
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct UpdatesResponse {
    pub data: Vec<UpdateCheckResult>,
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
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
            "mod_id": 10,
            "version_id": 20,
            "name": "ParentMod",
            "version": "1.0.0",
            "resolved_dependencies": [
                {
                    "mod_id": 30,
                    "version_id": 40,
                    "name": "ChildMod",
                    "version": "0.5.0",
                    "resolved_dependencies": [
                        {
                            "mod_id": 50,
                            "version_id": 60,
                            "name": "GrandchildMod",
                            "version": "0.1.0",
                            "resolved_dependencies": null
                        }
                    ]
                }
            ]
        }"#;

        let node: DependencyNode = serde_json::from_str(json).unwrap();
        assert_eq!(node.mod_id, 10);
        assert_eq!(node.version_id, 20);
        assert_eq!(node.name.as_deref(), Some("ParentMod"));

        let children = node.resolved_dependencies.expect("should have children");
        assert_eq!(children.len(), 1);
        assert_eq!(children[0].mod_id, 30);
        assert_eq!(children[0].name.as_deref(), Some("ChildMod"));

        let grandchildren = children[0]
            .resolved_dependencies
            .as_ref()
            .expect("should have grandchildren");
        assert_eq!(grandchildren.len(), 1);
        assert_eq!(grandchildren[0].mod_id, 50);
        assert_eq!(grandchildren[0].resolved_dependencies, None);
    }
}
