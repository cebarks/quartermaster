use std::path::Path;

use anyhow::{Context, Result};
use serde::Deserialize;

#[derive(Debug, Clone)]
pub struct SptRelease {
    pub version: String,
    pub eft_version: String,
    pub download_url: Option<String>,
}

#[derive(Deserialize)]
struct GitHubRelease {
    tag_name: String,
    body: Option<String>,
}

const RELEASES_URL: &str = "https://api.github.com/repos/SP-Tushonka/build/releases";

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
    let downloaded = download_release(release, on_progress).await?;
    downloaded.extract(dest_dir).await
}

pub struct DownloadedRelease {
    temp_dir: tempfile::TempDir,
}

impl DownloadedRelease {
    pub fn ensure_compatible(&self, dest: &Path) -> Result<()> {
        ensure_archive_layout_compatible(&self.temp_dir.path().join("spt-server.7z"), dest)
    }

    pub async fn extract(&self, dest: &Path) -> Result<()> {
        let archive = self.temp_dir.path().join("spt-server.7z");
        let dest = dest.to_path_buf();
        tokio::task::spawn_blocking(move || extract_spt_archive(&archive, &dest))
            .await
            .context("extraction task panicked")?
            .context("failed to extract SPT archive")
    }
}

/// Download to a temporary archive so update layout can be checked before stopping the server.
pub async fn download_release(
    release: &SptRelease,
    on_progress: impl Fn(u64, Option<u64>),
) -> Result<DownloadedRelease> {
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

    Ok(DownloadedRelease { temp_dir: tmp_dir })
}

fn safe_release_entry(entry: &sevenz_rust2::ArchiveEntry) -> Result<std::path::PathBuf> {
    let attributes = entry.windows_attributes();
    if entry.is_anti_item() || attributes & 0x0400 != 0 || (attributes >> 16) & 0o170000 == 0o120000
    {
        anyhow::bail!("archive contains symlink or anti-item: {}", entry.name());
    }
    super::scoped::relative_path(entry.name())
}

fn ensure_archive_layout_compatible(archive_path: &Path, dest: &Path) -> Result<()> {
    use super::layout::RuntimeLayout;
    let reader = sevenz_rust2::ArchiveReader::open(archive_path, sevenz_rust2::Password::empty())?;
    let scope = if dest.exists() {
        Some(super::scoped::ScopedDir::open(dest)?)
    } else {
        None
    };
    for entry in &reader.archive().files {
        let path = safe_release_entry(entry)?;
        if let Some(scope) = &scope {
            scope.resolve(&path)?;
        }
    }
    let names: std::collections::HashSet<String> = reader
        .archive()
        .files
        .iter()
        .filter(|entry| !entry.is_directory())
        .map(|entry| entry.name().replace('\\', "/"))
        .collect();
    let has_layout = |layout: RuntimeLayout| {
        names.contains(&format!("{}/SPT.Server.deps.json", layout.directory()))
            && names.contains(&format!(
                "{}/SPT_Data/configs/core.json",
                layout.directory()
            ))
    };
    let target = match (
        has_layout(RuntimeLayout::Current),
        has_layout(RuntimeLayout::Legacy),
    ) {
        (true, false) => RuntimeLayout::Current,
        (false, true) => RuntimeLayout::Legacy,
        _ => anyhow::bail!("SPT archive has missing or ambiguous runtime layout markers"),
    };
    let installed = super::layout::detect_layout(dest)?;
    let installed_runtime = dest.join(installed.directory());
    if installed_runtime.join("SPT.Server.deps.json").is_file() && target != installed {
        anyhow::bail!(
            "SPT update changes runtime layout from {} to {}; use a clean installation and explicitly migrate compatible mods/profiles before updating",
            installed.directory(), target.directory()
        );
    }
    Ok(())
}

/// Extract a .7z archive to `dest_dir`, preserving directory structure.
/// Security: rejects symlinks and path traversal.
fn extract_spt_archive(archive_path: &Path, dest_dir: &Path) -> Result<()> {
    use sevenz_rust2::{ArchiveReader, Password};

    ensure_archive_layout_compatible(archive_path, dest_dir)?;
    std::fs::create_dir_all(dest_dir).with_context(|| {
        format!(
            "failed to create destination directory {}",
            dest_dir.display()
        )
    })?;

    let scope = super::scoped::ScopedDir::open(dest_dir)?;
    let mut reader = ArchiveReader::open(archive_path, Password::empty())
        .with_context(|| format!("failed to open archive {}", archive_path.display()))?;

    reader
        .for_each_entries(|entry, reader| {
            let path = safe_release_entry(entry)
                .map_err(|e| sevenz_rust2::Error::Other(e.to_string().into()))?;
            if entry.is_directory() {
                scope
                    .create_dir_all(&path)
                    .map_err(|e| sevenz_rust2::Error::Other(e.to_string().into()))?;
                return Ok(true);
            }
            let mut file = scope
                .create_file(&path)
                .map_err(|e| sevenz_rust2::Error::Other(e.to_string().into()))?;
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
    fn release_absolute_entry_is_rejected_before_writes() {
        let outside = tempfile::NamedTempFile::new().unwrap();
        std::fs::write(outside.path(), b"secret").unwrap();
        let archive = crate::spt::mods::tests::create_test_7z(&[
            ("SPT_Runtime/SPT.Server.deps.json", b"{}"),
            ("SPT_Runtime/SPT_Data/configs/core.json", b"{}"),
            (outside.path().to_str().unwrap(), b"overwritten"),
        ]);
        let dest = tempfile::tempdir().unwrap();
        assert!(extract_spt_archive(archive.path(), dest.path()).is_err());
        assert_eq!(std::fs::read(outside.path()).unwrap(), b"secret");
        assert!(!dest.path().join("SPT_Runtime").exists());
    }

    #[cfg(unix)]
    #[test]
    fn release_outside_symlink_is_rejected_before_writes() {
        let outside = tempfile::NamedTempFile::new().unwrap();
        std::fs::write(outside.path(), b"secret").unwrap();
        let dest = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(dest.path().join("SPT_Runtime/SPT_Data/configs")).unwrap();
        std::fs::write(
            dest.path().join("SPT_Runtime/SPT.Server.deps.json"),
            b"old deps",
        )
        .unwrap();
        std::os::unix::fs::symlink(
            outside.path(),
            dest.path().join("SPT_Runtime/SPT_Data/configs/core.json"),
        )
        .unwrap();
        let archive = crate::spt::mods::tests::create_test_7z(&[
            ("SPT_Runtime/SPT.Server.deps.json", b"new deps"),
            ("SPT_Runtime/SPT_Data/configs/core.json", b"new core"),
        ]);
        assert!(extract_spt_archive(archive.path(), dest.path()).is_err());
        assert_eq!(std::fs::read(outside.path()).unwrap(), b"secret");
        assert_eq!(
            std::fs::read(dest.path().join("SPT_Runtime/SPT.Server.deps.json")).unwrap(),
            b"old deps"
        );
    }

    #[cfg(unix)]
    #[test]
    fn release_rejects_escaping_dangling_targets_before_writes() {
        let dest = tempfile::tempdir().unwrap();
        let outside = tempfile::tempdir().unwrap();
        let core = dest.path().join("SPT_Runtime/SPT_Data/configs/core.json");
        std::fs::create_dir_all(core.parent().unwrap()).unwrap();
        let deps = dest.path().join("SPT_Runtime/SPT.Server.deps.json");
        std::fs::write(&deps, b"old deps").unwrap();
        let target = outside.path().join("missing/config.json");
        std::os::unix::fs::symlink(&target, &core).unwrap();
        let archive = crate::spt::mods::tests::create_test_7z(&[
            ("SPT_Runtime/SPT.Server.deps.json", b"new deps"),
            ("SPT_Runtime/SPT_Data/configs/core.json", b"new core"),
        ]);
        assert!(extract_spt_archive(archive.path(), dest.path()).is_err());
        assert_eq!(std::fs::read(deps).unwrap(), b"old deps");
        assert_eq!(std::fs::read_dir(outside.path()).unwrap().count(), 0);
    }

    #[cfg(unix)]
    #[test]
    fn release_preserves_confined_symlinks() {
        for runtime in ["SPT_Runtime", "SPT"] {
            for absolute in [false, true] {
                let dest = tempfile::tempdir().unwrap();
                let deps = format!("{runtime}/SPT.Server.deps.json");
                let core = format!("{runtime}/SPT_Data/configs/core.json");
                std::fs::create_dir_all(dest.path().join(runtime).join("SPT_Data/configs"))
                    .unwrap();
                std::fs::write(dest.path().join(&deps), b"old deps").unwrap();
                let target = dest.path().join("actual-core.json");
                std::fs::write(&target, b"old core").unwrap();
                let link = if absolute {
                    target.clone()
                } else {
                    std::path::PathBuf::from("../../../actual-core.json")
                };
                std::os::unix::fs::symlink(link, dest.path().join(&core)).unwrap();
                let archive = crate::spt::mods::tests::create_test_7z(&[
                    (&deps, b"new deps"),
                    (&core, b"new core"),
                ]);
                extract_spt_archive(archive.path(), dest.path()).unwrap();
                assert!(std::fs::symlink_metadata(dest.path().join(&core))
                    .unwrap()
                    .file_type()
                    .is_symlink());
                assert_eq!(std::fs::read(target).unwrap(), b"new core");
            }
        }
    }

    #[cfg(unix)]
    #[test]
    fn release_creates_confined_dangling_targets() {
        for absolute in [false, true] {
            let dest = tempfile::tempdir().unwrap();
            std::fs::create_dir_all(dest.path().join("SPT_Runtime/SPT_Data/configs")).unwrap();
            std::fs::write(
                dest.path().join("SPT_Runtime/SPT.Server.deps.json"),
                b"old deps",
            )
            .unwrap();
            let target = dest.path().join("new/config.json");
            let link = if absolute {
                target.clone()
            } else {
                std::path::PathBuf::from("../../../new/config.json")
            };
            let core = dest.path().join("SPT_Runtime/SPT_Data/configs/core.json");
            std::os::unix::fs::symlink(link, &core).unwrap();
            let archive = crate::spt::mods::tests::create_test_7z(&[
                ("SPT_Runtime/SPT.Server.deps.json", b"new deps"),
                ("SPT_Runtime/SPT_Data/configs/core.json", b"new core"),
            ]);
            extract_spt_archive(archive.path(), dest.path()).unwrap();
            assert!(std::fs::symlink_metadata(core)
                .unwrap()
                .file_type()
                .is_symlink());
            assert_eq!(std::fs::read(target).unwrap(), b"new core");
        }
    }

    #[test]
    fn cross_layout_extraction_is_rejected_before_writes() {
        let archive = crate::spt::mods::tests::create_test_7z(&[
            ("SPT_Runtime/SPT.Server.deps.json", b"{}"),
            ("SPT_Runtime/SPT_Data/configs/core.json", b"{}"),
        ]);
        let dest = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(dest.path().join("SPT/SPT_Data/configs")).unwrap();
        std::fs::write(dest.path().join("SPT/SPT.Server.deps.json"), b"old deps").unwrap();
        std::fs::write(
            dest.path().join("SPT/SPT_Data/configs/core.json"),
            b"old core",
        )
        .unwrap();
        let error = extract_spt_archive(archive.path(), dest.path()).unwrap_err();
        assert!(error.to_string().contains("layout"));
        assert!(!dest.path().join("SPT_Runtime").exists());
        assert_eq!(
            std::fs::read(dest.path().join("SPT/SPT.Server.deps.json")).unwrap(),
            b"old deps"
        );
    }

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
    fn parse_release_spt_4_1_2() {
        let gh = GitHubRelease {
            tag_name: "4.1.2".to_string(),
            body: Some(
                concat!(
                    "#### Requires EFT `0.16.9.5.40743` (released 22nd October 2025)\r\n",
                    "\r\n## Direct Download\r\n",
                    "https://spt-releases.modd.in/SPT-4.1.2-40743-cf04a11.7z\r\n",
                    "md5/b64:  UVKk/gs/01Bhf+bnqKrzEw==\r\n",
                )
                .to_string(),
            ),
        };
        let release = parse_release(gh);
        assert_eq!(release.version, "4.1.2");
        assert_eq!(release.eft_version, "0.16.9.5.40743");
        assert_eq!(
            release.download_url.as_deref(),
            Some("https://spt-releases.modd.in/SPT-4.1.2-40743-cf04a11.7z")
        );
    }

    #[test]
    fn parse_release_spt_4_1_6() {
        let gh = GitHubRelease {
            tag_name: "4.1.6".to_string(),
            body: Some(
                concat!(
                    "#### Requires EFT `0.16.9.5.40743` (released 22nd October 2025)\r\n",
                    "\r\n## Direct Download\r\n",
                    "https://mirror.sp-tushonka.com/builds/SPT-4.1.6-40743-731d7a2.7z\r\n",
                    "md5/b64:  V0CKc6gSXCehdd5VQwsK3g==\r\n",
                )
                .to_string(),
            ),
        };
        let release = parse_release(gh);
        assert_eq!(release.version, "4.1.6");
        assert_eq!(release.eft_version, "0.16.9.5.40743");
        assert_eq!(
            release.download_url.as_deref(),
            Some("https://mirror.sp-tushonka.com/builds/SPT-4.1.6-40743-731d7a2.7z")
        );
    }

    #[test]
    fn parse_release_removed_download() {
        let gh = GitHubRelease {
            tag_name: "4.0.9".to_string(),
            body: Some("#### Requires EFT `0.16.9-40087`\n\n## Direct Download\nRemoved - Get latest instead\n".to_string()),
        };
        let release = parse_release(gh);
        assert_eq!(release.version, "4.0.9");
        assert!(release.download_url.is_none());
    }
}
