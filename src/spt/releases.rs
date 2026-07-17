use std::path::Path;

use anyhow::{Context, Result};
use serde::Deserialize;

#[derive(Debug, Clone)]
pub struct SptRelease {
    pub version: String,
    pub eft_version: String,
    pub download_url: Option<String>,
    pub tag: String,
    pub published_at: String,
}

#[derive(Deserialize)]
struct GitHubRelease {
    tag_name: String,
    body: Option<String>,
    published_at: Option<String>,
}

const RELEASES_URL: &str = "https://api.github.com/repos/sp-tarkov/build/releases";

/// Parse the download URL from the release body markdown.
/// Looks for `## Direct Download` followed by a URL on the next non-empty line.
/// Returns None if the section says "Removed" or is missing.
fn parse_download_url(body: &str) -> Option<String> {
    let mut in_section = false;
    for line in body.lines() {
        let trimmed = line.trim();
        if trimmed.eq_ignore_ascii_case("## direct download") {
            in_section = true;
            continue;
        }
        if in_section {
            if trimmed.is_empty() {
                continue;
            }
            if trimmed.starts_with("http://") || trimmed.starts_with("https://") {
                return Some(trimmed.to_string());
            }
            // "Removed - Get latest instead" or any non-URL text
            return None;
        }
    }
    None
}

/// Parse the EFT version from the release body.
/// Looks for `Requires EFT \`X.X.X-NNNNN\`` pattern.
fn parse_eft_version(body: &str) -> Option<String> {
    for line in body.lines() {
        // Pattern: "Requires EFT `0.16.9-40087`"
        if let Some(rest) = line.find("Requires EFT").and_then(|i| {
            let after = &line[i..];
            let start = after.find('`')? + 1;
            let end = start + after[start..].find('`')?;
            Some(after[start..end].to_string())
        }) {
            // Extract just the build number (e.g., "40087" from "0.16.9-40087")
            if let Some(num) = rest.rsplit('-').next() {
                return Some(num.to_string());
            }
            return Some(rest);
        }
    }
    None
}

fn parse_release(gh: GitHubRelease) -> SptRelease {
    let body = gh.body.as_deref().unwrap_or("");
    let version = gh
        .tag_name
        .strip_prefix('v')
        .unwrap_or(&gh.tag_name)
        .to_string();
    SptRelease {
        version,
        eft_version: parse_eft_version(body).unwrap_or_default(),
        download_url: parse_download_url(body),
        tag: gh.tag_name,
        published_at: gh.published_at.unwrap_or_default(),
    }
}

/// Fetch available SPT server releases from GitHub.
pub async fn list_releases() -> Result<Vec<SptRelease>> {
    let ua = format!("quartermaster/{}", env!("CARGO_PKG_VERSION"));
    let client = reqwest::Client::builder()
        .user_agent(&ua)
        .connect_timeout(std::time::Duration::from_secs(10))
        .timeout(std::time::Duration::from_secs(30))
        .build()
        .context("failed to build HTTP client")?;

    let resp = client
        .get(RELEASES_URL)
        .header("Accept", "application/vnd.github+json")
        .send()
        .await
        .context("failed to fetch SPT releases from GitHub")?;

    if resp.status() == reqwest::StatusCode::FORBIDDEN
        || resp.status() == reqwest::StatusCode::TOO_MANY_REQUESTS
    {
        anyhow::bail!("GitHub API rate limited. Try again later.");
    }

    let releases: Vec<GitHubRelease> = resp
        .error_for_status()
        .context("GitHub API returned an error")?
        .json()
        .await
        .context("failed to parse GitHub releases response")?;

    Ok(releases.into_iter().map(parse_release).collect())
}

/// Get the latest SPT release that has a valid download URL.
pub async fn get_latest_release() -> Result<SptRelease> {
    let releases = list_releases().await?;
    releases
        .into_iter()
        .find(|r| r.download_url.is_some())
        .ok_or_else(|| anyhow::anyhow!("no SPT releases with valid download URLs found"))
}

/// Download and extract an SPT server release to `dest_dir`.
///
/// The `.7z` archive is downloaded to a temp file, then extracted.
/// `on_progress` is called with (bytes_downloaded, total_bytes).
pub async fn download_and_extract_release(
    release: &SptRelease,
    dest_dir: &Path,
    on_progress: impl Fn(u64, Option<u64>),
) -> Result<()> {
    let url = release
        .download_url
        .as_deref()
        .ok_or_else(|| anyhow::anyhow!("release {} has no download URL", release.version))?;

    let ua = format!("quartermaster/{}", env!("CARGO_PKG_VERSION"));
    let client = reqwest::Client::builder()
        .user_agent(&ua)
        .connect_timeout(std::time::Duration::from_secs(30))
        .read_timeout(std::time::Duration::from_secs(120))
        .build()
        .context("failed to build HTTP client")?;

    let resp = client
        .get(url)
        .send()
        .await
        .context("failed to download SPT release")?
        .error_for_status()
        .with_context(|| format!("download failed for {url}"))?;

    let total = resp.content_length();

    // Stream to temp file
    let tmp_dir = tempfile::tempdir().context("failed to create temp directory")?;
    let archive_path = tmp_dir.path().join("spt-server.7z");

    {
        use futures_util::StreamExt;
        use tokio::io::AsyncWriteExt;

        let mut file = tokio::fs::File::create(&archive_path)
            .await
            .context("failed to create temp archive file")?;
        let mut downloaded: u64 = 0;
        let mut stream = resp.bytes_stream();

        while let Some(chunk) = stream.next().await {
            let chunk = chunk.context("error reading download stream")?;
            file.write_all(&chunk)
                .await
                .context("failed to write to temp file")?;
            downloaded += chunk.len() as u64;
            on_progress(downloaded, total);
        }
        file.flush().await?;
    }

    // Extract (blocking — 7z library is sync)
    tokio::task::spawn_blocking({
        let archive = archive_path.clone();
        let dest = dest_dir.to_path_buf();
        move || extract_spt_archive(&archive, &dest)
    })
    .await
    .context("extraction task panicked")?
    .context("failed to extract SPT archive")?;

    Ok(())
}

/// Extract a .7z archive to `dest_dir`, preserving directory structure.
/// Security: rejects symlinks and path traversal.
fn extract_spt_archive(archive_path: &Path, dest_dir: &Path) -> Result<()> {
    use sevenz_rust2::{ArchiveReader, Password};

    std::fs::create_dir_all(dest_dir).with_context(|| {
        format!(
            "failed to create destination directory {}",
            dest_dir.display()
        )
    })?;

    let mut reader = ArchiveReader::open(archive_path, Password::empty())
        .with_context(|| format!("failed to open archive {}", archive_path.display()))?;

    reader
        .for_each_entries(|entry, reader| {
            if entry.is_anti_item() {
                return Err(sevenz_rust2::Error::Other(
                    format!("archive contains anti-item: {}", entry.name()).into(),
                ));
            }

            // Reject symlinks (Windows reparse points)
            const FILE_ATTRIBUTE_REPARSE_POINT: u32 = 0x0400;
            if entry.windows_attributes() & FILE_ATTRIBUTE_REPARSE_POINT != 0 {
                return Err(sevenz_rust2::Error::Other(
                    format!("archive contains symlink entry: {}", entry.name()).into(),
                ));
            }

            let name = entry.name().replace('\\', "/");

            // Reject path traversal
            if name.contains("../") || name.contains("..\\") {
                return Err(sevenz_rust2::Error::Other(
                    format!("archive contains path traversal: {name}").into(),
                ));
            }

            let dest_path = dest_dir.join(&name);

            if entry.is_directory() {
                std::fs::create_dir_all(&dest_path).ok();
                return Ok(true);
            }

            if let Some(parent) = dest_path.parent() {
                std::fs::create_dir_all(parent).ok();
            }

            let mut file = std::fs::File::create(&dest_path).map_err(|e| {
                sevenz_rust2::Error::Io(
                    std::io::Error::new(
                        e.kind(),
                        format!("failed to create {}", dest_path.display()),
                    ),
                    "".into(),
                )
            })?;
            std::io::copy(reader, &mut file).map_err(|e| sevenz_rust2::Error::Io(e, "".into()))?;

            Ok(true)
        })
        .context("failed to extract archive entries")?;

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_download_url_valid() {
        let body = "Some text\n\n## Direct Download\nhttps://spt-releases.modd.in/SPT-4.0.13-40087-2891fd4.7z\n\n## Thanks\n";
        assert_eq!(
            parse_download_url(body),
            Some("https://spt-releases.modd.in/SPT-4.0.13-40087-2891fd4.7z".to_string())
        );
    }

    #[test]
    fn parse_download_url_removed() {
        let body = "## Direct Download\nRemoved - Get latest instead\n";
        assert_eq!(parse_download_url(body), None);
    }

    #[test]
    fn parse_download_url_missing_section() {
        let body = "## Release notes\nSome stuff\n";
        assert_eq!(parse_download_url(body), None);
    }

    #[test]
    fn parse_download_url_with_blank_lines() {
        let body = "## Direct Download\n\nhttps://example.com/file.7z\n";
        assert_eq!(
            parse_download_url(body),
            Some("https://example.com/file.7z".to_string())
        );
    }

    #[test]
    fn parse_eft_version_standard() {
        let body = "#### Requires EFT `0.16.9-40087` (released 12th September 2025)\n";
        assert_eq!(parse_eft_version(body), Some("40087".to_string()));
    }

    #[test]
    fn parse_eft_version_missing() {
        let body = "## Release notes\nStuff\n";
        assert_eq!(parse_eft_version(body), None);
    }

    #[test]
    fn parse_release_full() {
        let gh = GitHubRelease {
            tag_name: "4.0.13".to_string(),
            body: Some("#### Requires EFT `0.16.9-40087`\n\n## Direct Download\nhttps://spt-releases.modd.in/SPT-4.0.13-40087-2891fd4.7z\n".to_string()),
            published_at: Some("2026-01-15T00:00:00Z".to_string()),
        };
        let release = parse_release(gh);
        assert_eq!(release.version, "4.0.13");
        assert_eq!(release.eft_version, "40087");
        assert_eq!(
            release.download_url.as_deref(),
            Some("https://spt-releases.modd.in/SPT-4.0.13-40087-2891fd4.7z")
        );
    }

    #[test]
    fn parse_release_removed_download() {
        let gh = GitHubRelease {
            tag_name: "4.0.9".to_string(),
            body: Some("#### Requires EFT `0.16.9-40087`\n\n## Direct Download\nRemoved - Get latest instead\n".to_string()),
            published_at: Some("2025-12-25T00:00:00Z".to_string()),
        };
        let release = parse_release(gh);
        assert_eq!(release.version, "4.0.9");
        assert!(release.download_url.is_none());
    }
}
