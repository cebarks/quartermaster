//! GitHub release source for mods that are not on SPT Forge.
//!
//! quma already tracks non-Forge mods (`installed_mods.source = 'url'` +
//! `source_url`, added by migration 010) but nothing ever asked GitHub whether a
//! newer release existed. A release-download URL carries everything needed to do
//! that — owner, repo and the asset naming — so the update check is derived from
//! the URL we already store rather than a new source type.

use anyhow::{bail, Context, Result};

/// A GitHub release-download URL, split into the parts we need.
///
/// `https://github.com/{owner}/{repo}/releases/download/{tag}/{asset}`
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReleaseRef {
    pub owner: String,
    pub repo: String,
    pub tag: String,
    pub asset: String,
}

/// The newest release of a repo, and the asset to download for it.
#[derive(Debug, Clone)]
pub struct LatestRelease {
    pub version: String,
    pub download_url: String,
}

/// Parse a GitHub release-download URL. Returns `None` for any other URL, which
/// is how callers tell "this mod can be update-checked on GitHub" from "this mod
/// came from some other host".
pub fn parse_release_url(url: &str) -> Option<ReleaseRef> {
    let u = reqwest::Url::parse(url).ok()?;
    if u.host_str()? != "github.com" {
        return None;
    }
    let seg: Vec<&str> = u.path_segments()?.collect();
    // {owner}/{repo}/releases/download/{tag}/{asset}
    match seg.as_slice() {
        [owner, repo, "releases", "download", tag, asset] => Some(ReleaseRef {
            owner: (*owner).to_string(),
            repo: (*repo).to_string(),
            tag: (*tag).to_string(),
            asset: (*asset).to_string(),
        }),
        _ => None,
    }
}

/// Strip a leading `v` so tags and mod versions compare on equal terms
/// (`v0.12.5` and `0.12.5` are the same release).
pub fn normalize_version(s: &str) -> &str {
    s.strip_prefix('v').unwrap_or(s)
}

/// Ask GitHub for a repo's latest release and pick the asset to download.
///
/// Asset choice mirrors the one already installed: same name with the version
/// swapped. Falls back to the only `.zip` in the release when that name is not
/// found, and gives up if the release is ambiguous — guessing the wrong asset
/// would install the wrong thing.
pub async fn latest_release(r: &ReleaseRef) -> Result<LatestRelease> {
    #[derive(serde::Deserialize)]
    struct Asset {
        name: String,
        browser_download_url: String,
    }
    #[derive(serde::Deserialize)]
    struct Release {
        tag_name: String,
        #[serde(default)]
        draft: bool,
        #[serde(default)]
        prerelease: bool,
        #[serde(default)]
        assets: Vec<Asset>,
    }

    let url = format!(
        "https://api.github.com/repos/{}/{}/releases/latest",
        r.owner, r.repo
    );
    let client = reqwest::Client::builder()
        .connect_timeout(std::time::Duration::from_secs(15))
        .read_timeout(std::time::Duration::from_secs(30))
        .user_agent(concat!("quartermaster/", env!("CARGO_PKG_VERSION")))
        .build()
        .context("failed to build GitHub client")?;

    let rel: Release = client
        .get(&url)
        .header("Accept", "application/vnd.github+json")
        .send()
        .await
        .with_context(|| format!("GitHub request failed for {}/{}", r.owner, r.repo))?
        .error_for_status()
        .with_context(|| format!("GitHub returned an error for {}/{}", r.owner, r.repo))?
        .json()
        .await
        .context("failed to parse the GitHub release response")?;

    if rel.draft || rel.prerelease {
        bail!("latest GitHub release is a draft/prerelease — not offering it as an update");
    }

    let version = normalize_version(&rel.tag_name).to_string();
    let want = r.asset.replace(normalize_version(&r.tag), &version);

    let asset = rel
        .assets
        .iter()
        .find(|a| a.name == want)
        .or_else(|| {
            let zips: Vec<&Asset> = rel
                .assets
                .iter()
                .filter(|a| a.name.ends_with(".zip"))
                .collect();
            match zips.as_slice() {
                [only] => Some(*only),
                _ => None,
            }
        })
        .with_context(|| {
            format!(
                "release {} has no asset named {want} and no single .zip to fall back to",
                rel.tag_name
            )
        })?;

    Ok(LatestRelease {
        version,
        download_url: asset.browser_download_url.clone(),
    })
}

// ponytail: one request per GitHub repo per 15 min — only a handful of mods are
// GitHub-sourced. Revisit only if someone runs dozens of them.
const RELEASE_TTL: std::time::Duration = std::time::Duration::from_secs(900);

type ReleaseCache = parking_lot::Mutex<
    std::collections::HashMap<String, (std::time::Instant, Option<LatestRelease>)>,
>;

fn cache() -> &'static ReleaseCache {
    static CACHE: std::sync::OnceLock<ReleaseCache> = std::sync::OnceLock::new();
    CACHE.get_or_init(Default::default)
}

/// `latest_release`, memoised. Returns `None` when there is no answer to give —
/// callers treat that as "no update known", never as an error worth failing on.
pub async fn latest_release_cached(r: &ReleaseRef) -> Option<LatestRelease> {
    let key = format!("{}/{}", r.owner, r.repo);
    let hit = cache().lock().get(&key).cloned();
    if let Some((at, cached)) = hit {
        if at.elapsed() < RELEASE_TTL {
            return cached;
        }
    }
    let fresh = latest_release(r).await.ok();
    cache()
        .lock()
        .insert(key, (std::time::Instant::now(), fresh.clone()));
    fresh
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_a_release_download_url() {
        let r = parse_release_url(
            "https://github.com/Dildz/ModSync-for-SPT4.0/releases/download/v0.12.5/Corter-ModSync-v0.12.5.zip",
        )
        .expect("should parse");
        assert_eq!(r.owner, "Dildz");
        assert_eq!(r.repo, "ModSync-for-SPT4.0");
        assert_eq!(r.tag, "v0.12.5");
        assert_eq!(r.asset, "Corter-ModSync-v0.12.5.zip");
    }

    #[test]
    fn rejects_non_release_and_non_github_urls() {
        assert!(parse_release_url("https://example.com/mod.zip").is_none());
        assert!(parse_release_url("https://github.com/Dildz/ModSync-for-SPT4.0").is_none());
        assert!(parse_release_url("not a url").is_none());
    }

    #[test]
    fn version_normalizes_across_the_v_prefix() {
        assert_eq!(normalize_version("v0.12.5"), "0.12.5");
        assert_eq!(normalize_version("0.12.5"), "0.12.5");
    }
}
