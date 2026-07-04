use crate::config::{Config, HeadlessConfig, HeadlessDisplayServer, HeadlessRunner};
use crate::container::{
    ContainerManager, CreateContainerOpts, PortMapping, Protocol, SelinuxLabel, VolumeMount,
};
use crate::forge::client::ForgeClient;
use crate::spt::profiles;
use crate::spt::server::SptClient;
use anyhow::{bail, Context, Result};
use bollard::models::DeviceMapping;
use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, LazyLock};
use tracing::{debug, info, warn};

struct ConvergingGuard(Arc<AtomicBool>);

impl Drop for ConvergingGuard {
    fn drop(&mut self) {
        self.0.store(false, Ordering::Release);
    }
}

/// Label key for marking containers as managed by quartermaster
pub const MANAGED_BY_LABEL: &str = "quma.managed-by";
pub const MANAGED_BY_VALUE: &str = "quartermaster-clients";

use crate::config::FIKA_CLIENT_FORGE_ID;

/// Regex for editing headless amount in fika.jsonc
static AMOUNT_RE: LazyLock<regex::Regex> =
    LazyLock::new(|| regex::Regex::new(r#"("amount"\s*:\s*)\d+"#).expect("valid regex"));

/// Regex for editing UDP port in Fika client config
static PORT_RE: LazyLock<regex::Regex> =
    LazyLock::new(|| regex::Regex::new(r"(?m)^(Port\s*=\s*)\d+").expect("valid regex"));

/// Edit the headless amount in fika.jsonc using targeted text replacement to preserve comments.
///
/// This function does NOT parse the full JSON - it uses a regex to find and replace only the
/// numeric value of the "amount" key, leaving all comments and formatting intact.
pub fn edit_headless_amount(path: &Path, amount: u32) -> Result<()> {
    if !path.exists() {
        bail!(
            "Fika server mod not configured. Start the SPT server at least once to generate fika.jsonc, then retry."
        );
    }

    let content = std::fs::read_to_string(path)
        .with_context(|| format!("failed to read {}", path.display()))?;

    if !AMOUNT_RE.is_match(&content) {
        bail!("could not find headless.amount in {}", path.display());
    }

    let updated = AMOUNT_RE.replace(&content, format!("${{1}}{amount}"));
    std::fs::write(path, updated.as_ref())
        .with_context(|| format!("failed to write {}", path.display()))?;

    debug!("Updated headless amount to {amount} in {}", path.display());
    Ok(())
}

/// Check if the Fika client mod is present in the headless install directory.
///
/// Looks for a `Fika.Core.dll`, `Fika.Core/`, or `Fika/` entry in
/// `install_dir/BepInEx/plugins/`. The archive layout has changed across
/// Fika versions (older: `Fika.Core/`, newer: `Fika/`), so we check both.
pub fn is_fika_client_present(install_dir: &Path) -> bool {
    let plugins_dir = install_dir.join("BepInEx/plugins");
    let Ok(entries) = std::fs::read_dir(&plugins_dir) else {
        return false;
    };
    entries.flatten().any(|e| {
        let name = e.file_name();
        let s = name.to_string_lossy();
        s == "Fika" || s.starts_with("Fika.Core")
    })
}

/// Ensure the Fika client mod is installed in the headless install directory.
///
/// If Fika.Core is already present, this is a no-op. Otherwise, it queries the Forge API
/// for the latest compatible version, downloads the archive to a temp file, and extracts
/// it into the install directory.
pub async fn ensure_fika_client(
    forge: &ForgeClient,
    install_dir: &Path,
    spt_version: &str,
    spt_client: &SptClient,
) -> Result<()> {
    // Detect the server's Fika version so we can match the client to it.
    // The container image may auto-update the server mod independently of
    // what Forge reports as "latest compatible".
    let server_fika_version = spt_client.loaded_server_mods().await.ok().and_then(|mods| {
        mods.get("server")
            .and_then(|m| m.get("Version"))
            .and_then(|v| v.as_str().map(String::from))
    });

    let fika_present = is_fika_client_present(install_dir);

    if fika_present && server_fika_version.is_none() {
        debug!("Fika client already present in {}", install_dir.display());
        return Ok(());
    }

    // If the server is running a known Fika version and we already have the
    // client installed, check whether the installed version matches.  We
    // store the installed version in a marker file next to the DLL.
    if fika_present {
        if let Some(ref target) = server_fika_version {
            let marker = install_dir.join("BepInEx/plugins/Fika/.fika-client-version");
            let installed = std::fs::read_to_string(&marker).unwrap_or_default();
            if installed.trim() == target.as_str() {
                debug!("Fika client v{target} already matches server");
                return Ok(());
            }
            info!(
                "Fika client version mismatch (installed: {}, server: {target}) — updating",
                if installed.trim().is_empty() {
                    "unknown"
                } else {
                    installed.trim()
                }
            );
            // Remove the old client so we can install the matching version
            let fika_dir = install_dir.join("BepInEx/plugins/Fika");
            if let Err(e) = std::fs::remove_dir_all(&fika_dir) {
                warn!("Failed to remove old Fika client: {e}");
            }
        }
    }

    info!(
        "Fika client not found or outdated in {}. Resolving version...",
        install_dir.display()
    );

    // Try Forge first, fall back to GitHub releases if the server's version
    // isn't on Forge (common when the container image auto-updates Fika).
    let (download_url, resolved_version) =
        resolve_fika_client_download(forge, spt_version, server_fika_version.as_deref()).await?;

    info!("Downloading Fika client v{resolved_version}");

    let tmp_file = tempfile::NamedTempFile::new().context("failed to create temp file")?;
    forge
        .download_file(&download_url, tmp_file.path())
        .await
        .context("failed to download Fika client archive")?;

    crate::spt::mods::extract_mod(tmp_file.path(), install_dir)
        .context("failed to extract Fika client archive")?;

    let marker = install_dir.join("BepInEx/plugins/Fika/.fika-client-version");
    let _ = std::fs::write(&marker, &resolved_version);

    info!(
        "Fika client v{resolved_version} installed to {}",
        install_dir.display()
    );
    Ok(())
}

const FIKA_CLIENT_GITHUB_REPO: &str = "project-fika/Fika-Plugin";
const FIKA_HEADLESS_GITHUB_REPO: &str = "project-fika/Fika-Headless";

async fn resolve_fika_client_download(
    forge: &ForgeClient,
    spt_version: &str,
    server_fika_version: Option<&str>,
) -> Result<(String, String)> {
    // Try Forge first
    if let Ok(versions) = forge
        .get_versions(FIKA_CLIENT_FORGE_ID, Some(spt_version))
        .await
    {
        let matched = if let Some(target) = server_fika_version {
            versions
                .iter()
                .find(|v| v.version == target)
                .or_else(|| versions.iter().max_by_key(|v| v.id))
        } else {
            versions.iter().max_by_key(|v| v.id)
        };

        if let Some(v) = matched {
            let version_matches_server =
                server_fika_version.map(|t| v.version == t).unwrap_or(true);
            if version_matches_server {
                if let Some(ref url) = v.link {
                    return Ok((url.clone(), v.version.clone()));
                }
            }
        }
    }

    // Fall back to GitHub releases if Forge doesn't have the server's version
    if let Some(target) = server_fika_version {
        info!("Forge doesn't have Fika v{target}, trying GitHub releases...");
        let client = reqwest::Client::builder()
            .user_agent("quartermaster")
            .timeout(std::time::Duration::from_secs(30))
            .build()
            .context("failed to build HTTP client")?;

        let release: serde_json::Value = client
            .get(format!(
                "https://api.github.com/repos/{FIKA_CLIENT_GITHUB_REPO}/releases/tags/v{target}"
            ))
            .send()
            .await
            .context("failed to query GitHub for Fika client release")?
            .error_for_status()
            .context("GitHub API returned error for Fika v{target}")?
            .json()
            .await
            .context("failed to parse GitHub release response")?;

        let asset = release["assets"]
            .as_array()
            .and_then(|a| a.first())
            .ok_or_else(|| anyhow::anyhow!("Fika v{target} GitHub release has no assets"))?;
        let url = asset["browser_download_url"]
            .as_str()
            .ok_or_else(|| anyhow::anyhow!("Fika v{target} asset has no download URL"))?;

        return Ok((url.to_string(), target.to_string()));
    }

    bail!("no Fika client version found on Forge or GitHub for SPT {spt_version}")
}

/// Check if the Fika.Headless plugin is present in the install directory.
///
/// This is a separate plugin from Fika.Core — it implements the headless idle
/// loop that keeps the game running and ready to host raids.
fn is_fika_headless_present(install_dir: &Path) -> bool {
    install_dir
        .join("BepInEx/plugins/Fika/Fika.Headless.dll")
        .is_file()
}

/// Ensure the Fika.Headless plugin is installed in the headless install directory.
///
/// Unlike Fika.Core (which is on Forge), Fika.Headless is distributed via GitHub
/// releases at project-fika/Fika-Headless. This function fetches the latest release,
/// downloads the zip, and extracts it.
async fn ensure_fika_headless(forge: &ForgeClient, install_dir: &Path) -> Result<()> {
    if is_fika_headless_present(install_dir) {
        debug!("Fika.Headless already present in {}", install_dir.display());
        return Ok(());
    }

    info!(
        "Fika.Headless not found in {}. Downloading from GitHub...",
        install_dir.display()
    );

    // Query GitHub API for the latest release
    let client = reqwest::Client::builder()
        .user_agent("quartermaster")
        .timeout(std::time::Duration::from_secs(30))
        .build()
        .context("failed to build HTTP client")?;

    let release: serde_json::Value = client
        .get(format!(
            "https://api.github.com/repos/{FIKA_HEADLESS_GITHUB_REPO}/releases/latest"
        ))
        .send()
        .await
        .context("failed to query GitHub for Fika.Headless releases")?
        .error_for_status()
        .context("GitHub API returned error")?
        .json()
        .await
        .context("failed to parse GitHub release response")?;

    let tag = release["tag_name"].as_str().unwrap_or("unknown");
    let asset = release["assets"]
        .as_array()
        .and_then(|a| a.first())
        .ok_or_else(|| anyhow::anyhow!("Fika.Headless latest release has no assets"))?;
    let download_url = asset["browser_download_url"]
        .as_str()
        .ok_or_else(|| anyhow::anyhow!("Fika.Headless asset has no download URL"))?;

    info!("Downloading Fika.Headless {tag}...");

    let tmp_file = tempfile::NamedTempFile::new().context("failed to create temp file")?;
    forge
        .download_file(download_url, tmp_file.path())
        .await
        .context("failed to download Fika.Headless archive")?;

    crate::spt::mods::extract_mod(tmp_file.path(), install_dir)
        .context("failed to extract Fika.Headless archive")?;

    info!("Fika.Headless {tag} installed to {}", install_dir.display());
    Ok(())
}

/// Discover new profile IDs that appeared after a baseline snapshot.
///
/// Compares the current set of .json files in the profiles directory against a baseline
/// set of profile IDs. Returns the profile IDs (filenames without .json extension) that
/// are new.
#[cfg(test)]
#[allow(clippy::unwrap_used)]
pub fn discover_new_profiles(profiles_dir: &Path, before: &HashSet<String>) -> Vec<String> {
    let mut new_profiles = Vec::new();

    if let Ok(entries) = std::fs::read_dir(profiles_dir) {
        for entry in entries.flatten() {
            if let Some(name) = entry.file_name().to_str() {
                if name.ends_with(".json") {
                    let profile_id = name.trim_end_matches(".json");
                    if !before.contains(profile_id) {
                        new_profiles.push(profile_id.to_string());
                    }
                }
            }
        }
    }

    new_profiles.sort();
    new_profiles
}

/// Collect profile IDs already assigned to running managed containers via their PROFILE_ID env var.
async fn assigned_profile_ids(
    container_mgr: &ContainerManager,
    managed_names: &[String],
) -> HashSet<String> {
    let mut assigned = HashSet::new();
    for name in managed_names {
        if let Ok(info) = container_mgr.inspect(name).await {
            if let Some(pid) = info.config.and_then(|c| c.env).and_then(|env| {
                env.iter()
                    .find(|e| e.starts_with("PROFILE_ID="))
                    .and_then(|e| e.strip_prefix("PROFILE_ID="))
                    .map(String::from)
            }) {
                assigned.insert(pid);
            }
        }
    }
    assigned
}

/// Select profile IDs to assign to new containers during scale-up.
///
/// Reads profiles from `<spt_dir>/SPT/user/profiles/`, filters out those already
/// assigned to running containers, and prefers profiles whose username starts with
/// `headless_` (Fika's auto-generated headless profiles). Returns up to `needed`
/// profile IDs.
async fn select_profiles_for_assignment(
    container_mgr: &ContainerManager,
    spt_dir: &Path,
    managed_names: &[String],
    needed: u32,
) -> Vec<Option<String>> {
    let already_assigned = assigned_profile_ids(container_mgr, managed_names).await;

    let all_profiles = match profiles::list_profiles(spt_dir) {
        Ok(p) => p,
        Err(e) => {
            warn!("Failed to read profiles directory: {e}. Containers will be created without PROFILE_ID.");
            return vec![None; needed as usize];
        }
    };

    // Filter to unassigned profiles, preferring headless_ prefixed usernames
    let mut headless_profiles: Vec<String> = Vec::new();
    let mut other_profiles: Vec<String> = Vec::new();

    for profile in &all_profiles {
        if already_assigned.contains(&profile.aid) {
            continue;
        }
        if profile.username.starts_with("headless_") {
            headless_profiles.push(profile.aid.clone());
        } else {
            other_profiles.push(profile.aid.clone());
        }
    }

    // Headless profiles first, then other profiles as fallback
    let available: Vec<String> = headless_profiles
        .into_iter()
        .chain(other_profiles)
        .collect();

    let mut assignments: Vec<Option<String>> = Vec::with_capacity(needed as usize);
    for i in 0..needed as usize {
        if i < available.len() {
            assignments.push(Some(available[i].clone()));
        } else {
            assignments.push(None);
        }
    }

    let assigned_count = assignments.iter().filter(|a| a.is_some()).count();
    if assigned_count < needed as usize {
        warn!(
            "Only {} of {} profiles available for assignment. \
             Remaining containers will be created without PROFILE_ID. \
             Headless profiles are generated when the SPT server starts with Fika — \
             restart the server to create them, then run `quma headless create` again.",
            assigned_count, needed
        );
    }

    assignments
}

/// Write the UDP port into the Fika client config file in a per-client overlay.
///
/// The Fika client reads its P2P port from `BepInEx/config/com.fika.core.cfg`
/// under the `[Network]` section, `Port = <value>`. This function uses regex
/// replacement to update the port value while preserving comments and formatting.
///
/// If the config file doesn't exist yet (overlay was just created), this is a no-op —
/// the config will be populated when Fika runs for the first time.
fn write_fika_udp_port(overlay_dir: &Path, port: u16) -> Result<()> {
    let cfg_path = overlay_dir.join("BepInEx/config/com.fika.core.cfg");
    if !cfg_path.exists() {
        debug!(
            "Fika config not found at {}, skipping UDP port write",
            cfg_path.display()
        );
        return Ok(());
    }

    let content = std::fs::read_to_string(&cfg_path)
        .with_context(|| format!("failed to read {}", cfg_path.display()))?;

    if !PORT_RE.is_match(&content) {
        debug!("No Port setting found in {}, skipping", cfg_path.display());
        return Ok(());
    }

    let updated = PORT_RE
        .replace(&content, format!("${{1}}{port}"))
        .to_string();
    std::fs::write(&cfg_path, &updated)
        .with_context(|| format!("failed to write {}", cfg_path.display()))?;

    debug!("Set UDP port to {port} in {}", cfg_path.display());
    Ok(())
}

/// Set up overlay directory for a client, copying isolated paths from the install directory.
///
/// Creates `<install_dir>/.quma/clients/<index>/` and recursively copies any paths from
/// `isolated_paths` that don't already exist in the overlay. This preserves user
/// customizations in the overlay while ensuring new isolated paths are populated.
///
/// Overlays live under `install_dir` (the game client directory) rather than `spt_dir`
/// (the SPT server data directory) when possible. Note: `install_dir` may still be
/// inside `spt_dir` — the wine-prefix mount uses `:z` (Shared) instead of `:Z`
/// (Private) to avoid SELinux MCS conflicts with the SPT server container's chown.
pub fn setup_client_overlay(
    install_dir: &Path,
    index: u32,
    isolated_paths: &[String],
    base_udp_port: u16,
) -> Result<()> {
    let overlay_dir = client_overlay_dir(install_dir, index);

    let wine_prefix_dir = overlay_dir.join("wine-prefix");
    if !wine_prefix_dir.exists() {
        std::fs::create_dir_all(&wine_prefix_dir).with_context(|| {
            format!(
                "failed to create wine-prefix dir {}",
                wine_prefix_dir.display()
            )
        })?;
        debug!("Created wine-prefix directory for client {index}");
    }

    ensure_wine_registry(&wine_prefix_dir)?;

    for isolated_path in isolated_paths {
        let src = install_dir.join(isolated_path);
        let dst = overlay_dir.join(isolated_path);

        // Skip if destination already exists (preserve user customizations)
        if dst.exists() {
            continue;
        }

        // Ensure parent directory exists
        if let Some(parent) = dst.parent() {
            std::fs::create_dir_all(parent)
                .with_context(|| format!("failed to create parent dir {}", parent.display()))?;
        }

        // Copy recursively if source is a directory, or just copy the file
        if src.is_dir() {
            copy_dir_recursive(&src, &dst).with_context(|| {
                format!("failed to copy {} to {}", src.display(), dst.display())
            })?;
        } else if src.is_file() {
            std::fs::copy(&src, &dst).with_context(|| {
                format!("failed to copy {} to {}", src.display(), dst.display())
            })?;
        } else {
            debug!(
                "Isolated path {} does not exist in install dir, skipping",
                isolated_path
            );
        }
    }

    let port = client_port(base_udp_port, index);
    write_fika_udp_port(&overlay_dir, port)?;

    // Stub out GPU-specific DLLs that crash in headless (no-GPU) containers.
    // DLSSImporter.dll dereferences a null pointer when no GPU is present,
    // which hangs Wine's crash handler and prevents the game from starting.
    stub_gpu_dlls(install_dir, &overlay_dir)?;

    debug!(
        "Set up overlay for client {index} at {}",
        overlay_dir.display()
    );
    Ok(())
}

/// Ensure the wine prefix `user.reg` has the registry keys EFT needs under Proton:
///
/// 1. `winhttp=native,builtin` DLL override — BepInEx uses a `winhttp.dll` doorstop
///    to inject itself. Without this override Wine uses its builtin winhttp.dll and
///    BepInEx never loads.
///
/// 2. `ProxyEnable=0` under Internet Settings — Mono's `AutoWebProxyScriptEngine`
///    reads Windows proxy config from the registry. When the key is absent (fresh
///    wine prefix), it dereferences null and crashes every HTTP request, blocking
///    SPT's client patches from initializing.
///
/// The baked container image includes these in its `/.wine` prefix, but quma's
/// per-client wine prefix overlay hides them. This function seeds or patches the
/// overlay's `user.reg` so they survive.
fn ensure_wine_registry(wine_prefix_dir: &Path) -> Result<()> {
    let user_reg = wine_prefix_dir.join("user.reg");

    let winhttp_entry = r#""winhttp"="native,builtin""#;
    let proxy_section = "[Software\\\\Microsoft\\\\Windows\\\\CurrentVersion\\\\Internet Settings]";
    let proxy_entry = "\"ProxyEnable\"=dword:00000000";

    if user_reg.exists() {
        let content = std::fs::read_to_string(&user_reg)
            .with_context(|| format!("failed to read {}", user_reg.display()))?;

        let needs_winhttp = !content.contains(winhttp_entry);
        let needs_proxy = !content.contains(proxy_entry);

        if !needs_winhttp && !needs_proxy {
            return Ok(());
        }

        let mut patched = content;

        if needs_winhttp {
            if let Some(pos) = patched.find("[Software\\\\Wine\\\\DllOverrides]") {
                let insert_at = patched[pos..]
                    .find('\n')
                    .map(|i| pos + i + 1)
                    .unwrap_or(patched.len());
                patched.insert_str(insert_at, &format!("{winhttp_entry}\n"));
            } else {
                patched.push_str(&format!(
                    "\n[Software\\\\Wine\\\\DllOverrides]\n{winhttp_entry}\n\n"
                ));
            }
        }

        if needs_proxy {
            patched.push_str(&format!("\n{proxy_section}\n{proxy_entry}\n\n"));
        }

        std::fs::write(&user_reg, patched)
            .with_context(|| format!("failed to patch {}", user_reg.display()))?;
        debug!("Patched wine registry in {}", user_reg.display());
    } else {
        let seed = format!(
            "WINE REGISTRY Version 2\n\
             ;; All keys relative to \\\\Registry\\\\User\\\\S-1-5-21-0-0-0-1000\n\
             \n\
             #arch=win64\n\
             \n\
             [Software\\\\Wine\\\\DllOverrides]\n\
             {winhttp_entry}\n\
             \n\
             {proxy_section}\n\
             {proxy_entry}\n\
             \n"
        );
        std::fs::write(&user_reg, seed)
            .with_context(|| format!("failed to seed {}", user_reg.display()))?;
        debug!("Seeded wine registry in {}", user_reg.display());
    }

    Ok(())
}

/// Replace GPU-specific DLLs with empty stubs in the overlay so Unity
/// skips them at load time instead of crashing in headless containers.
fn stub_gpu_dlls(install_dir: &Path, overlay_dir: &Path) -> Result<()> {
    let gpu_dlls = ["EscapeFromTarkov_Data/Plugins/x86_64/DLSSImporter.dll"];

    for rel_path in &gpu_dlls {
        let src = install_dir.join(rel_path);
        if !src.exists() {
            continue;
        }

        let dst = overlay_dir.join(rel_path);
        if dst.exists() {
            continue;
        }

        if let Some(parent) = dst.parent() {
            std::fs::create_dir_all(parent)
                .with_context(|| format!("failed to create dir {}", parent.display()))?;
        }

        // Write a zero-byte stub — Unity will fail to load it as a
        // native plugin and log a warning instead of crashing.
        std::fs::write(&dst, b"").with_context(|| format!("failed to stub {}", dst.display()))?;

        debug!("Stubbed GPU DLL for headless: {rel_path}");
    }

    Ok(())
}

/// Recursively copy a directory. Symlinks are skipped to prevent cycles
/// and avoid copying sensitive files outside the intended tree.
fn copy_dir_recursive(src: &Path, dst: &Path) -> Result<()> {
    std::fs::create_dir_all(dst)
        .with_context(|| format!("failed to create dir {}", dst.display()))?;

    for entry in
        std::fs::read_dir(src).with_context(|| format!("failed to read dir {}", src.display()))?
    {
        let entry = entry?;
        let file_type = entry.file_type()?;
        if file_type.is_symlink() {
            tracing::debug!(path = %entry.path().display(), "skipping symlink during directory copy");
            continue;
        }

        let src_path = entry.path();
        let dst_path = dst.join(entry.file_name());

        if file_type.is_dir() {
            copy_dir_recursive(&src_path, &dst_path)?;
        } else {
            std::fs::copy(&src_path, &dst_path)?;
        }
    }

    Ok(())
}

/// Generate the container name for a client at the given index.
pub fn client_container_name(index: u32) -> String {
    format!("fika-headless-{index}")
}

/// Return the overlay directory for a client at the given index.
pub fn client_overlay_dir(install_dir: &Path, index: u32) -> PathBuf {
    install_dir.join(".quma/clients").join(index.to_string())
}

/// Calculate the UDP port for a client at the given index.
///
/// Ports are assigned sequentially starting from base_udp_port:
/// - index 1 → base_udp_port
/// - index 2 → base_udp_port + 1
/// - index 3 → base_udp_port + 2
pub fn client_port(base: u16, index: u32) -> u16 {
    base + (index - 1) as u16
}

/// Find containers that match the naming pattern but lack the managed-by label.
///
/// This detects name collisions: containers that have names like "fika-headless-N" but
/// were created outside of quartermaster's management. These would conflict with the
/// containers we want to create.
///
/// Returns the names of conflicting containers.
#[cfg(test)]
#[allow(clippy::unwrap_used)]
pub fn find_name_conflicts(
    managed: &[String],
    all_matching_name: &[String],
    desired_count: u32,
) -> Vec<String> {
    let managed_set: HashSet<_> = managed.iter().collect();
    let mut conflicts = Vec::new();

    // Check all containers that match the naming pattern
    for name in all_matching_name {
        if !managed_set.contains(name) {
            conflicts.push(name.clone());
        }
    }

    // Also check if any of the desired names would conflict
    for i in 1..=desired_count {
        let name = client_container_name(i);
        if all_matching_name.contains(&name)
            && !managed_set.contains(&name)
            && !conflicts.contains(&name)
        {
            conflicts.push(name);
        }
    }

    conflicts.sort();
    conflicts.dedup();
    conflicts
}

/// Main convergence function: reconcile desired client count with actual state.
///
/// This function:
/// 1. Sets the converging flag to prevent concurrent modifications
/// 2. Detects currently managed containers by label
/// 3. Checks for name conflicts with unmanaged containers
/// 4. Scales up or down as needed to match desired count
/// 5. Updates isolated_paths overlays for existing clients
/// 6. Clears the converging flag
///
/// The `converging` flag is an Arc<AtomicBool> that prevents concurrent convergence
/// operations and signals to the supervisor that state is in flux.
#[allow(clippy::too_many_arguments)]
pub async fn converge(
    container_mgr: &ContainerManager,
    headless_config: &HeadlessConfig,
    config: &Config,
    spt_dir: &Path,
    spt_client: &SptClient,
    forge: &ForgeClient,
    spt_version: &str,
    converging: Arc<AtomicBool>,
) -> Result<()> {
    // Set converging flag (atomic compare-exchange for race-free check-and-set)
    if converging
        .compare_exchange(false, true, Ordering::Acquire, Ordering::Relaxed)
        .is_err()
    {
        bail!("Convergence already in progress");
    }

    let _guard = ConvergingGuard(converging.clone());

    let ntsync_available = std::path::Path::new("/dev/ntsync").exists();
    if !ntsync_available {
        tracing::warn!(
            "ntsync not available — headless clients will run with degraded performance. \
             Run `sudo modprobe ntsync` or upgrade to kernel 6.14+."
        );
    }

    let desired_count = headless_config.client_count();
    info!("Starting convergence: desired count = {desired_count}");

    // Detect currently managed containers
    let managed = container_mgr
        .detect_containers_by_label(MANAGED_BY_LABEL, MANAGED_BY_VALUE)
        .await?;

    debug!("Found {} managed containers", managed.len());

    // Check for name conflicts with unmanaged containers
    // For each desired container name, check if it exists but isn't in our managed list
    let mut conflicts = Vec::new();
    for i in 1..=desired_count {
        let name = client_container_name(i);
        // Try to inspect the container - if it exists but isn't in managed list, it's a conflict
        if container_mgr.inspect(&name).await.is_ok() && !managed.contains(&name) {
            conflicts.push(name);
        }
    }

    if !conflicts.is_empty() {
        bail!(
            "Cannot converge: the following containers have conflicting names but are not managed by quartermaster: {}. \
            Please remove or rename them manually.",
            conflicts.join(", ")
        );
    }

    // Determine current count
    let current_count = managed.len() as u32;

    // Scale up or down
    if current_count < desired_count {
        // Ensure the Fika client mod is installed before creating containers
        ensure_fika_client(forge, &headless_config.install_dir, spt_version, spt_client).await?;
        ensure_fika_headless(forge, &headless_config.install_dir).await?;

        ensure_clients(
            container_mgr,
            headless_config,
            config,
            spt_dir,
            spt_client,
            current_count,
            desired_count,
            ntsync_available,
        )
        .await?;
    } else if current_count > desired_count {
        remove_excess_clients(
            container_mgr,
            config,
            spt_dir,
            spt_client,
            current_count,
            desired_count,
        )
        .await?;
    } else {
        info!("Already at desired count ({desired_count}), checking for overlay updates");
    }

    // Update overlays for all defined clients (using effective paths)
    for (i, _client_def) in headless_config.clients.iter().enumerate() {
        let index = (i + 1) as u32;
        let effective_paths = headless_config.effective_isolated_paths(i);
        setup_client_overlay(
            &headless_config.install_dir,
            index,
            &effective_paths,
            headless_config.base_udp_port,
        )?;
    }

    info!("Convergence complete");
    Ok(())
}

/// Poll the SPT server until it responds to ping, or until timeout.
///
/// Returns `true` if the server became ready, `false` on timeout.
async fn await_server_ready(spt_client: &SptClient, timeout_secs: u64) -> bool {
    let deadline = tokio::time::Instant::now() + std::time::Duration::from_secs(timeout_secs);
    loop {
        // Check loaded mods, not just ping — the server responds to ping
        // before all mods are loaded and endpoints are ready. Headless
        // clients that connect too early hit Connection refused on
        // endpoints that BepInEx patches depend on.
        if spt_client.loaded_server_mods().await.is_ok() {
            return true;
        }
        if tokio::time::Instant::now() >= deadline {
            return false;
        }
        tokio::time::sleep(std::time::Duration::from_secs(2)).await;
    }
}

/// Restart all running managed headless containers so they reconnect to a fresh SPT server.
///
/// After the SPT server is restarted (for config changes, scale-up, or scale-down),
/// existing headless containers retain stale connections and won't appear in the Fika
/// headless API. Restarting them forces a fresh connection to the new server instance.
async fn restart_running_clients(container_mgr: &ContainerManager, count: u32) -> Result<()> {
    let futs: Vec<_> = (1..=count)
        .map(|i| {
            let name = client_container_name(i);
            async move {
                match container_mgr.is_running(&name).await {
                    Ok(true) => {
                        info!("Restarting {name} to reconnect to fresh SPT server");
                        if let Err(e) = container_mgr.restart(&name).await {
                            warn!("Failed to restart {name} after server restart: {e}");
                            return Some(format!("{name}: {e}"));
                        }
                    }
                    Ok(false) => {
                        debug!("Skipping restart of {name} (not running)");
                    }
                    Err(e) => {
                        warn!("Failed to check status of {name}: {e}");
                        return Some(format!("{name}: {e}"));
                    }
                }
                None
            }
        })
        .collect();

    let results = futures_util::future::join_all(futs).await;
    let errors: Vec<_> = results.into_iter().flatten().collect();

    if errors.is_empty() {
        Ok(())
    } else {
        bail!(
            "Failed to restart {} client(s): {}",
            errors.len(),
            errors.join("; ")
        )
    }
}

/// Ensure containers exist from current_count up to desired_count.
#[allow(clippy::too_many_arguments)]
async fn ensure_clients(
    container_mgr: &ContainerManager,
    headless_config: &HeadlessConfig,
    config: &Config,
    spt_dir: &Path,
    spt_client: &SptClient,
    current_count: u32,
    desired_count: u32,
    ntsync_available: bool,
) -> Result<()> {
    info!("Ensuring clients: {current_count} → {desired_count}");

    // 1. Edit fika.jsonc to set amount
    let fika_config_path = spt_dir.join("SPT/user/mods/fika-server/assets/configs/fika.jsonc");
    edit_headless_amount(&fika_config_path, desired_count)?;

    // 2. Restart SPT server to pick up new headless count and generate profiles
    let container = config
        .server_container
        .as_deref()
        .expect("server_container validated by HeadlessConfig::validate");

    info!("Stopping SPT server");
    container_mgr
        .stop(container)
        .await
        .context("failed to stop SPT server for headless config update")?;

    info!("Starting SPT server");
    container_mgr
        .start(container)
        .await
        .context("failed to start SPT server after headless config update")?;

    info!("Waiting for SPT server to become ready");
    if !await_server_ready(spt_client, 120).await {
        warn!(
            "SPT server did not respond within 120s after restart. \
             Proceeding with profile discovery — profiles may not be available yet."
        );
    }

    // 3. Restart existing containers so they reconnect to the fresh server
    restart_running_clients(container_mgr, current_count).await?;

    // 4. Discover available profiles for assignment
    // Headless profiles are created by the SPT server when it starts with Fika's
    // headless.amount > 0. If profiles don't exist yet (server hasn't been restarted
    // since headless amount was increased), containers are created without PROFILE_ID
    // and will need a re-scale after the server generates the profiles.
    let new_count = desired_count - current_count;
    let managed = container_mgr
        .detect_containers_by_label(MANAGED_BY_LABEL, MANAGED_BY_VALUE)
        .await
        .unwrap_or_default();
    let profile_assignments =
        select_profiles_for_assignment(container_mgr, spt_dir, &managed, new_count).await;

    // 5. Create containers for new clients
    for (offset, profile_id) in profile_assignments.into_iter().enumerate() {
        let i = current_count + 1 + offset as u32;
        let client_index = (i - 1) as usize;
        let effective_paths = headless_config.effective_isolated_paths(client_index);
        create_client_container(
            container_mgr,
            headless_config,
            config,
            i,
            profile_id,
            &effective_paths,
            ntsync_available,
        )
        .await?;
    }

    Ok(())
}

/// Remove containers above desired_count.
///
/// In-raid checks are now handled by the CLI `headless delete` command before calling
/// converge, so this function simply stops and removes excess containers.
async fn remove_excess_clients(
    container_mgr: &ContainerManager,
    config: &Config,
    spt_dir: &Path,
    spt_client: &SptClient,
    current_count: u32,
    desired_count: u32,
) -> Result<()> {
    info!("Removing excess clients: {current_count} → {desired_count}");

    // 1. Remove containers for clients above desired_count
    for i in (desired_count + 1)..=current_count {
        let name = client_container_name(i);
        info!("Removing container {name}");

        // Stop first if running
        if container_mgr.is_running(&name).await? {
            container_mgr.stop(&name).await?;
        }

        container_mgr.remove_container(&name).await?;
    }

    // 2. Edit fika.jsonc to set amount
    let fika_config_path = spt_dir.join("SPT/user/mods/fika-server/assets/configs/fika.jsonc");
    edit_headless_amount(&fika_config_path, desired_count)?;

    // 3. Restart SPT server to deregister removed clients
    let container = config
        .server_container
        .as_deref()
        .expect("server_container validated by HeadlessConfig::validate");

    info!("Restarting SPT server to deregister removed headless clients");
    container_mgr
        .stop(container)
        .await
        .context("failed to stop SPT server for client deregistration")?;
    container_mgr
        .start(container)
        .await
        .context("failed to start SPT server after client deregistration")?;

    // 4. Wait for server readiness before restarting remaining clients
    info!("Waiting for SPT server to become ready");
    if !await_server_ready(spt_client, 120).await {
        warn!(
            "SPT server did not respond within 120s after restart. \
             Remaining clients may not reconnect immediately."
        );
    }

    // 5. Restart remaining clients so they reconnect to the fresh server
    restart_running_clients(container_mgr, desired_count).await?;

    Ok(())
}

/// Create a container for a single client.
///
/// If `profile_id` is `Some`, the container gets a `PROFILE_ID` env var that allows
/// the CLI status command and supervisor to correlate this container with the Fika
/// headless API (which keys its response by profile/session ID).
///
/// `effective_paths` is the merged list of global `isolated_paths` + per-client
/// `extra_isolated_paths`, computed by the caller via `HeadlessConfig::effective_isolated_paths`.
#[allow(clippy::too_many_arguments)]
async fn create_client_container(
    container_mgr: &ContainerManager,
    headless_config: &HeadlessConfig,
    config: &Config,
    index: u32,
    profile_id: Option<String>,
    effective_paths: &[String],
    ntsync_available: bool,
) -> Result<()> {
    let name = client_container_name(index);
    let overlay_dir = client_overlay_dir(&headless_config.install_dir, index);

    // Set up overlay directory first
    setup_client_overlay(
        &headless_config.install_dir,
        index,
        effective_paths,
        headless_config.base_udp_port,
    )?;

    // Build volume mounts
    let mut volumes = vec![
        // Game client directory — mounted read-write because the headless image
        // entrypoint writes wine.log, BepInEx logs, etc. directly into this tree.
        VolumeMount {
            host_path: headless_config.install_dir.clone(),
            container_path: "/opt/tarkov".to_string(),
            read_only: false,
            selinux: SelinuxLabel::Shared,
        },
    ];

    // Mount each isolated path from the overlay on top of the base install,
    // so per-client config/state shadows the shared copy.
    for isolated_path in effective_paths {
        let p = std::path::Path::new(isolated_path);
        if p.is_absolute() || p.components().any(|c| c == std::path::Component::ParentDir) {
            bail!("isolated_path contains traversal sequence: {isolated_path:?}");
        }
        let overlay_subdir = overlay_dir.join(isolated_path);
        let container_subdir = format!("/opt/tarkov/{isolated_path}");
        volumes.push(VolumeMount {
            host_path: overlay_subdir,
            container_path: container_subdir,
            read_only: false,
            selinux: SelinuxLabel::Shared,
        });
    }

    // ponytail: Shared not Private — `:Z` gives each headless container unique MCS
    // categories, but the SPT server entrypoint chowns the whole spt_dir tree and
    // can't traverse dirs relabeled by a different container's `:Z`.
    volumes.push(VolumeMount {
        host_path: overlay_dir.join("wine-prefix"),
        container_path: "/.wine".to_string(),
        read_only: false,
        selinux: SelinuxLabel::Shared,
    });

    // Mount stubbed GPU DLLs over the originals so headless clients
    // don't crash from missing GPU hardware.
    let gpu_stub = overlay_dir.join("EscapeFromTarkov_Data/Plugins/x86_64/DLSSImporter.dll");
    if gpu_stub.exists() {
        volumes.push(VolumeMount {
            host_path: gpu_stub,
            container_path: "/opt/tarkov/EscapeFromTarkov_Data/Plugins/x86_64/DLSSImporter.dll"
                .to_string(),
            read_only: true,
            selinux: SelinuxLabel::Shared,
        });
    }

    // Environment variables
    let mut env = vec![(
        "UDP_PORT".to_string(),
        client_port(headless_config.base_udp_port, index).to_string(),
    )];

    // Always set PROFILE_ID — the claudeoris image defaults to "test" if unset,
    // which would silently connect with a bogus profile. An empty string
    // causes the entrypoint to fail clearly instead.
    match profile_id {
        Some(ref pid) => {
            env.push(("PROFILE_ID".to_string(), pid.clone()));
            info!("Assigning profile {pid} to client {index}");
        }
        None => {
            env.push(("PROFILE_ID".to_string(), String::new()));
            warn!(
                "No profile available for client {index}. \
                 Fika status correlation will not work until a profile is assigned. \
                 Restart the SPT server to generate headless profiles, then re-scale."
            );
        }
    }

    // Route through quma's HTTPS proxy
    let proxy_host = match config.web_bind.as_str() {
        "0.0.0.0" | "127.0.0.1" | "localhost" | "" => "host.containers.internal",
        other => other,
    };
    env.push(("SERVER_URL".to_string(), proxy_host.to_string()));
    env.push(("SERVER_PORT".to_string(), config.web_port.to_string()));

    // Claudeoris image runtime knobs
    let runner_str = match headless_config.runner {
        HeadlessRunner::Umu => "umu",
        HeadlessRunner::Wine => "wine",
    };
    env.push(("RUNNER".to_string(), runner_str.to_string()));

    let effective_ntsync = ntsync_available && headless_config.ntsync;
    env.push(("NTSYNC".to_string(), effective_ntsync.to_string()));
    env.push(("ESYNC".to_string(), headless_config.esync.to_string()));
    env.push(("FSYNC".to_string(), headless_config.fsync.to_string()));

    let display_server_str = match headless_config.display_server {
        HeadlessDisplayServer::Gamescope => "gamescope",
        HeadlessDisplayServer::Xvfb => "xvfb",
    };
    env.push(("DISPLAY_SERVER".to_string(), display_server_str.to_string()));
    env.push((
        "SAVE_LOG_ON_EXIT".to_string(),
        headless_config.save_log_on_exit.to_string(),
    ));
    env.push((
        "ENABLE_LOG_PURGE".to_string(),
        headless_config.enable_log_purge.to_string(),
    ));
    env.push((
        "OVERWRITE_FIKA".to_string(),
        headless_config.overwrite_fika.to_string(),
    ));

    // Labels
    let labels = vec![
        (MANAGED_BY_LABEL.to_string(), MANAGED_BY_VALUE.to_string()),
        ("quma.client.index".to_string(), index.to_string()),
    ];

    let udp_port = client_port(headless_config.base_udp_port, index);
    let ports = vec![PortMapping {
        host_port: udp_port,
        container_port: udp_port,
        protocol: Protocol::Udp,
    }];

    let mut devices = vec![];
    if ntsync_available {
        devices.push(DeviceMapping {
            path_on_host: Some("/dev/ntsync".to_string()),
            path_in_container: Some("/dev/ntsync".to_string()),
            cgroup_permissions: Some("rwm".to_string()),
        });
    }

    // Pass through DRI render nodes so the EFT Unity engine can initialize.
    // Without a GPU device, Unity hangs during graphics init even with -nographics.
    for entry in std::fs::read_dir("/dev/dri")
        .into_iter()
        .flatten()
        .flatten()
    {
        let name = entry.file_name();
        let name = name.to_string_lossy();
        if name.starts_with("card") || name.starts_with("renderD") {
            let path = entry.path().to_string_lossy().to_string();
            devices.push(DeviceMapping {
                path_on_host: Some(path.clone()),
                path_in_container: Some(path),
                cgroup_permissions: Some("rwm".to_string()),
            });
        }
    }

    let security_opt = if devices.is_empty() {
        vec![]
    } else {
        vec!["label=disable".to_string()]
    };

    // Create the container
    let opts = CreateContainerOpts {
        name: name.clone(),
        image: headless_config.image.clone(),
        env,
        volumes,
        ports,
        labels,
        user: None,
        healthcheck: None,
        devices,
        security_opt,
    };

    let container_id = container_mgr.create_container(opts).await?;
    info!("Created container {name} (id: {container_id})");

    // Start the container
    container_mgr.start(&name).await?;
    info!("Started container {name}");

    Ok(())
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;
    use std::collections::HashSet;

    #[test]
    fn edit_headless_amount_preserves_comments() {
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("fika.jsonc");
        std::fs::write(
            &path,
            r#"{
    // This is a comment about headless settings
    "headless": {
        "amount": 1, // number of headless clients
        "profiles": {}
    }
}"#,
        )
        .unwrap();

        edit_headless_amount(&path, 3).unwrap();

        let content = std::fs::read_to_string(&path).unwrap();
        assert!(content.contains(r#""amount": 3"#));
        assert!(content.contains("// This is a comment"));
        assert!(content.contains("// number of headless"));
    }

    #[test]
    fn edit_headless_amount_no_file() {
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("fika.jsonc");
        let result = edit_headless_amount(&path, 3);
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("not configured"));
    }

    #[test]
    fn discover_new_profiles_finds_diff() {
        let tmp = tempfile::tempdir().unwrap();
        let profiles_dir = tmp.path();
        std::fs::write(profiles_dir.join("existing123.json"), "{}").unwrap();
        std::fs::write(profiles_dir.join("new456.json"), "{}").unwrap();

        let before: HashSet<String> = ["existing123".to_string()].into_iter().collect();
        let new = discover_new_profiles(profiles_dir, &before);
        assert_eq!(new, vec!["new456"]);
    }

    #[test]
    fn setup_client_overlay_copies_isolated_paths() {
        let tmp = tempfile::tempdir().unwrap();
        let install_dir = tmp.path().join("install");

        std::fs::create_dir_all(install_dir.join("BepInEx/config")).unwrap();
        std::fs::write(install_dir.join("BepInEx/config/test.cfg"), "key=value").unwrap();

        setup_client_overlay(&install_dir, 1, &["BepInEx/config".to_string()], 25565).unwrap();

        let overlay_file = install_dir.join(".quma/clients/1/BepInEx/config/test.cfg");
        assert!(overlay_file.exists());
        assert_eq!(std::fs::read_to_string(overlay_file).unwrap(), "key=value");
    }

    #[test]
    fn container_name_for_index() {
        assert_eq!(client_container_name(1), "fika-headless-1");
        assert_eq!(client_container_name(10), "fika-headless-10");
    }

    #[test]
    fn client_udp_port() {
        assert_eq!(client_port(25565, 1), 25565);
        assert_eq!(client_port(25565, 3), 25567);
    }

    #[test]
    fn setup_client_overlay_creates_wine_prefix_dir() {
        let tmp = tempfile::tempdir().unwrap();
        let install_dir = tmp.path().join("install");

        std::fs::create_dir_all(install_dir.join("BepInEx/config")).unwrap();

        setup_client_overlay(&install_dir, 1, &["BepInEx/config".to_string()], 25565).unwrap();

        let wine_prefix_dir = install_dir.join(".quma/clients/1/wine-prefix");
        assert!(wine_prefix_dir.exists());
        assert!(wine_prefix_dir.is_dir());
    }

    #[test]
    fn container_name_collision_detected() {
        let managed = vec!["fika-headless-1".to_string()];
        let all_matching_name = vec!["fika-headless-1".to_string(), "fika-headless-2".to_string()];
        let conflicts = find_name_conflicts(&managed, &all_matching_name, 3);
        // fika-headless-2 exists but isn't managed, fika-headless-3 doesn't exist
        assert_eq!(conflicts, vec!["fika-headless-2"]);
    }

    #[test]
    fn update_overlay_copies_new_paths() {
        let tmp = tempfile::tempdir().unwrap();
        let install_dir = tmp.path().join("install");

        // Existing overlay from initial setup
        std::fs::create_dir_all(install_dir.join("BepInEx/config")).unwrap();
        std::fs::write(install_dir.join("BepInEx/config/test.cfg"), "key=value").unwrap();
        setup_client_overlay(&install_dir, 1, &["BepInEx/config".to_string()], 25565).unwrap();

        // Now add a new isolated path
        std::fs::create_dir_all(install_dir.join("BepInEx/cache")).unwrap();
        std::fs::write(install_dir.join("BepInEx/cache/data.bin"), "cached").unwrap();
        setup_client_overlay(
            &install_dir,
            1,
            &["BepInEx/config".to_string(), "BepInEx/cache".to_string()],
            25565,
        )
        .unwrap();

        // Both paths should exist in overlay
        assert!(install_dir
            .join(".quma/clients/1/BepInEx/config/test.cfg")
            .exists());
        assert!(install_dir
            .join(".quma/clients/1/BepInEx/cache/data.bin")
            .exists());
    }

    #[test]
    fn fika_client_detected_by_dll() {
        let tmp = tempfile::tempdir().unwrap();
        let plugins = tmp.path().join("BepInEx/plugins");
        std::fs::create_dir_all(&plugins).unwrap();
        std::fs::write(plugins.join("Fika.Core.dll"), b"fake dll").unwrap();

        assert!(is_fika_client_present(tmp.path()));
    }

    #[test]
    fn fika_client_detected_by_directory() {
        let tmp = tempfile::tempdir().unwrap();
        let fika_dir = tmp.path().join("BepInEx/plugins/Fika.Core");
        std::fs::create_dir_all(&fika_dir).unwrap();
        std::fs::write(fika_dir.join("Fika.Core.dll"), b"fake dll").unwrap();

        assert!(is_fika_client_present(tmp.path()));
    }

    #[test]
    fn fika_client_not_detected_when_absent() {
        let tmp = tempfile::tempdir().unwrap();
        let plugins = tmp.path().join("BepInEx/plugins");
        std::fs::create_dir_all(&plugins).unwrap();
        std::fs::write(plugins.join("SomeOtherMod.dll"), b"other").unwrap();

        assert!(!is_fika_client_present(tmp.path()));
    }

    #[test]
    fn fika_client_not_detected_no_plugins_dir() {
        let tmp = tempfile::tempdir().unwrap();
        // No BepInEx/plugins directory at all
        assert!(!is_fika_client_present(tmp.path()));
    }

    #[test]
    fn fika_client_detected_by_fika_directory() {
        let tmp = tempfile::tempdir().unwrap();
        let fika_dir = tmp.path().join("BepInEx/plugins/Fika");
        std::fs::create_dir_all(&fika_dir).unwrap();
        std::fs::write(fika_dir.join("Fika.Core.dll"), b"fake dll").unwrap();

        assert!(is_fika_client_present(tmp.path()));
    }

    #[test]
    fn fika_compat_mod_not_false_positive() {
        let tmp = tempfile::tempdir().unwrap();
        let plugins = tmp.path().join("BepInEx/plugins");
        std::fs::create_dir_all(&plugins).unwrap();
        // "Fika.Compat" should NOT trigger detection — only "Fika.Core" matters
        std::fs::write(plugins.join("Fika.Compat.dll"), b"compat mod").unwrap();

        assert!(!is_fika_client_present(tmp.path()));
    }

    #[test]
    fn write_fika_udp_port_updates_config() {
        let tmp = tempfile::tempdir().unwrap();
        let config_dir = tmp.path().join("BepInEx/config");
        std::fs::create_dir_all(&config_dir).unwrap();

        let cfg_path = config_dir.join("com.fika.core.cfg");
        std::fs::write(
            &cfg_path,
            "[Network]\n\n## Port\n# Setting type: UInt16\n# Default value: 25565\nPort = 25565\n",
        )
        .unwrap();

        write_fika_udp_port(tmp.path(), 25567).unwrap();

        let content = std::fs::read_to_string(&cfg_path).unwrap();
        assert!(content.contains("Port = 25567"));
        assert!(!content.contains("Port = 25565"));
    }

    #[test]
    fn write_fika_udp_port_no_config_file_is_ok() {
        let tmp = tempfile::tempdir().unwrap();
        // No config file exists — should not error, just skip
        let result = write_fika_udp_port(tmp.path(), 25567);
        assert!(result.is_ok());
    }
}
