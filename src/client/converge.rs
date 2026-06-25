use crate::config::{Config, HeadlessConfig};
use crate::container::{ContainerManager, CreateContainerOpts, SelinuxLabel, VolumeMount};
use crate::forge::client::ForgeClient;
use crate::spt::profiles;
use crate::spt::server::SptClient;
use anyhow::{bail, Context, Result};
use std::collections::HashSet;
use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use tracing::{debug, info, warn};

/// Label key for marking containers as managed by quartermaster
const MANAGED_BY_LABEL: &str = "quma.managed-by";
const MANAGED_BY_VALUE: &str = "quartermaster-clients";

/// Forge mod ID for the Fika client mod (https://forge.sp-tarkov.com/mod/2326)
const FIKA_CLIENT_FORGE_ID: i64 = 2326;

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

    // Pattern: "amount" followed by optional whitespace, colon, optional whitespace, then digits
    let re = regex::Regex::new(r#"("amount"\s*:\s*)\d+"#).expect("valid regex");

    if !re.is_match(&content) {
        bail!("could not find headless.amount in {}", path.display());
    }

    let updated = re.replace(&content, format!("${{1}}{amount}"));
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
) -> Result<()> {
    if is_fika_client_present(install_dir) {
        debug!("Fika client already present in {}", install_dir.display());
        return Ok(());
    }

    info!(
        "Fika client not found in {}. Downloading from Forge...",
        install_dir.display()
    );

    // Find a compatible version for the current SPT version
    let versions = forge
        .get_versions(FIKA_CLIENT_FORGE_ID, Some(spt_version))
        .await
        .context("failed to query Forge for Fika client versions")?;

    let version = versions
        .into_iter()
        .max_by_key(|v| v.id)
        .ok_or_else(|| {
            anyhow::anyhow!(
                "no Fika client version found for SPT {spt_version} on Forge (mod ID {FIKA_CLIENT_FORGE_ID})"
            )
        })?;

    let download_url = version.link.as_deref().ok_or_else(|| {
        anyhow::anyhow!(
            "Fika client version {} has no download link",
            version.version
        )
    })?;

    info!(
        "Downloading Fika client v{} for SPT {spt_version}",
        version.version
    );

    // Download to a temp file, then extract
    let tmp_file = tempfile::NamedTempFile::new().context("failed to create temp file")?;
    forge
        .download_file(download_url, tmp_file.path())
        .await
        .context("failed to download Fika client archive")?;

    crate::spt::mods::extract_mod(tmp_file.path(), install_dir)
        .context("failed to extract Fika client archive")?;

    info!(
        "Fika client v{} installed to {}",
        version.version,
        install_dir.display()
    );
    Ok(())
}

const FIKA_HEADLESS_GITHUB_REPO: &str = "project-fika/Fika-Headless";

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

/// Set up overlay directory for a client, copying isolated paths from the install directory.
///
/// Creates `<spt_dir>/clients/<index>/` and recursively copies any paths from `isolated_paths`
/// that don't already exist in the overlay. This preserves user customizations in the overlay
/// while ensuring new isolated paths are populated.
pub fn setup_client_overlay(
    spt_dir: &Path,
    index: u32,
    install_dir: &Path,
    isolated_paths: &[String],
) -> Result<()> {
    let overlay_dir = spt_dir.join("clients").join(index.to_string());

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

    debug!(
        "Set up overlay for client {index} at {}",
        overlay_dir.display()
    );
    Ok(())
}

/// Recursively copy a directory.
fn copy_dir_recursive(src: &Path, dst: &Path) -> Result<()> {
    std::fs::create_dir_all(dst)
        .with_context(|| format!("failed to create dir {}", dst.display()))?;

    for entry in
        std::fs::read_dir(src).with_context(|| format!("failed to read dir {}", src.display()))?
    {
        let entry = entry?;
        let src_path = entry.path();
        let dst_path = dst.join(entry.file_name());

        if src_path.is_dir() {
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

    // Ensure converging flag is cleared on exit
    let _guard = scopeguard::guard(converging.clone(), |c| {
        c.store(false, Ordering::Release);
    });

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
        ensure_fika_client(forge, &headless_config.install_dir, spt_version).await?;
        ensure_fika_headless(forge, &headless_config.install_dir).await?;

        ensure_clients(
            container_mgr,
            headless_config,
            config,
            spt_dir,
            spt_client,
            current_count,
            desired_count,
        )
        .await?;
    } else if current_count > desired_count {
        remove_excess_clients(container_mgr, config, spt_dir, current_count, desired_count).await?;
    } else {
        info!("Already at desired count ({desired_count}), checking for overlay updates");
    }

    // Update overlays for all defined clients (using effective paths)
    for (i, _client_def) in headless_config.clients.iter().enumerate() {
        let index = (i + 1) as u32;
        let effective_paths = headless_config.effective_isolated_paths(i);
        setup_client_overlay(
            spt_dir,
            index,
            &headless_config.install_dir,
            &effective_paths,
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
        if let Ok(result) = spt_client.ping().await {
            if result.ok {
                return true;
            }
        }
        if tokio::time::Instant::now() >= deadline {
            return false;
        }
        tokio::time::sleep(std::time::Duration::from_secs(2)).await;
    }
}

/// Ensure containers exist from current_count up to desired_count.
async fn ensure_clients(
    container_mgr: &ContainerManager,
    headless_config: &HeadlessConfig,
    config: &Config,
    spt_dir: &Path,
    spt_client: &SptClient,
    current_count: u32,
    desired_count: u32,
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

    // 3. Discover available profiles for assignment
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

    // 4. Create containers for new clients
    for (offset, profile_id) in profile_assignments.into_iter().enumerate() {
        let i = current_count + 1 + offset as u32;
        let client_index = (i - 1) as usize;
        let effective_paths = headless_config.effective_isolated_paths(client_index);
        create_client_container(
            container_mgr,
            headless_config,
            config,
            spt_dir,
            i,
            profile_id,
            &effective_paths,
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
async fn create_client_container(
    container_mgr: &ContainerManager,
    headless_config: &HeadlessConfig,
    config: &Config,
    spt_dir: &Path,
    index: u32,
    profile_id: Option<String>,
    effective_paths: &[String],
) -> Result<()> {
    let name = client_container_name(index);
    let overlay_dir = spt_dir.join("clients").join(index.to_string());

    // Set up overlay directory first
    setup_client_overlay(
        spt_dir,
        index,
        &headless_config.install_dir,
        effective_paths,
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

    // Environment variables
    let mut env = vec![
        ("HEADLESS_INDEX".to_string(), index.to_string()),
        (
            "UDP_PORT".to_string(),
            client_port(headless_config.base_udp_port, index).to_string(),
        ),
    ];

    // Add PROFILE_ID for Fika API correlation
    if let Some(ref pid) = profile_id {
        env.push(("PROFILE_ID".to_string(), pid.clone()));
        info!("Assigning profile {pid} to client {index}");
    } else {
        warn!(
            "No profile available for client {index}. \
             Fika status correlation will not work until a profile is assigned. \
             Restart the SPT server to generate headless profiles, then re-scale."
        );
    }

    // SERVER_URL / SERVER_PORT are what the headless image reads to connect.
    // When the server binds 0.0.0.0, use host.containers.internal so the
    // headless container can reach the host's network stack via Podman DNS.
    let server_host = config
        .server_host
        .as_deref()
        .unwrap_or("host.containers.internal");
    let server_url = match server_host {
        "0.0.0.0" | "127.0.0.1" | "localhost" => "host.containers.internal",
        other => other,
    };
    env.push(("SERVER_URL".to_string(), server_url.to_string()));
    env.push((
        "SERVER_PORT".to_string(),
        config.server_port.unwrap_or(6969).to_string(),
    ));

    // Labels
    let labels = vec![
        (MANAGED_BY_LABEL.to_string(), MANAGED_BY_VALUE.to_string()),
        ("quma.client.index".to_string(), index.to_string()),
    ];

    // Create the container
    let opts = CreateContainerOpts {
        name: name.clone(),
        image: headless_config.image.clone(),
        env,
        volumes,
        ports: vec![],
        labels,
        user: None,
        healthcheck: None,
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
        let spt_dir = tmp.path().join("spt");
        let install_dir = tmp.path().join("install");

        std::fs::create_dir_all(install_dir.join("BepInEx/config")).unwrap();
        std::fs::write(install_dir.join("BepInEx/config/test.cfg"), "key=value").unwrap();

        setup_client_overlay(&spt_dir, 1, &install_dir, &["BepInEx/config".to_string()]).unwrap();

        let overlay_file = spt_dir.join("clients/1/BepInEx/config/test.cfg");
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
        let spt_dir = tmp.path().join("spt");
        let install_dir = tmp.path().join("install");

        // Existing overlay from initial setup
        std::fs::create_dir_all(install_dir.join("BepInEx/config")).unwrap();
        std::fs::write(install_dir.join("BepInEx/config/test.cfg"), "key=value").unwrap();
        setup_client_overlay(&spt_dir, 1, &install_dir, &["BepInEx/config".to_string()]).unwrap();

        // Now add a new isolated path
        std::fs::create_dir_all(install_dir.join("BepInEx/cache")).unwrap();
        std::fs::write(install_dir.join("BepInEx/cache/data.bin"), "cached").unwrap();
        setup_client_overlay(
            &spt_dir,
            1,
            &install_dir,
            &["BepInEx/config".to_string(), "BepInEx/cache".to_string()],
        )
        .unwrap();

        // Both paths should exist in overlay
        assert!(spt_dir.join("clients/1/BepInEx/config/test.cfg").exists());
        assert!(spt_dir.join("clients/1/BepInEx/cache/data.bin").exists());
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
}
