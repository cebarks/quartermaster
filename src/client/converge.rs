use crate::config::{ClientsConfig, Config};
use crate::container::{ContainerManager, CreateContainerOpts, SelinuxLabel, VolumeMount};
use crate::spt::server::SptClient;
use anyhow::{bail, Context, Result};
use std::collections::HashSet;
use std::path::Path;
use std::sync::Arc;
use tokio::sync::RwLock;
use tracing::{debug, info};

/// Label key for marking containers as managed by quartermaster
const MANAGED_BY_LABEL: &str = "quma.managed-by";
const MANAGED_BY_VALUE: &str = "quartermaster-clients";

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

/// Discover new profile IDs that appeared after a baseline snapshot.
///
/// Compares the current set of .json files in the profiles directory against a baseline
/// set of profile IDs. Returns the profile IDs (filenames without .json extension) that
/// are new.
#[allow(dead_code)] // Used only in tests
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
#[allow(dead_code)] // Used only in tests
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
/// The `converging` flag is an Arc<RwLock<bool>> that prevents concurrent convergence
/// operations and signals to the supervisor that state is in flux.
pub async fn converge(
    container_mgr: &ContainerManager,
    clients_config: &ClientsConfig,
    config: &Config,
    spt_dir: &Path,
    spt_client: &SptClient,
    converging: Arc<RwLock<bool>>,
) -> Result<()> {
    // Set converging flag
    {
        let mut flag = converging.write().await;
        if *flag {
            bail!("Convergence already in progress");
        }
        *flag = true;
    }

    // Ensure converging flag is cleared on exit
    let _guard = scopeguard::guard(converging.clone(), |c| {
        tokio::spawn(async move {
            *c.write().await = false;
        });
    });

    let desired_count = clients_config.count;
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
        scale_up(
            container_mgr,
            clients_config,
            config,
            spt_dir,
            spt_client,
            current_count,
            desired_count,
        )
        .await?;
    } else if current_count > desired_count {
        scale_down(
            container_mgr,
            clients_config,
            spt_dir,
            spt_client,
            current_count,
            desired_count,
            false, // force = false (CLI will call confirm(), web will pass force param)
        )
        .await?;
    } else {
        info!("Already at desired count ({desired_count}), checking for overlay updates");
    }

    // Update overlays for all existing clients (in case isolated_paths changed)
    for i in 1..=desired_count {
        setup_client_overlay(
            spt_dir,
            i,
            &clients_config.install_dir,
            &clients_config.isolated_paths,
        )?;
    }

    info!("Convergence complete");
    Ok(())
}

/// Scale up from current_count to desired_count.
async fn scale_up(
    container_mgr: &ContainerManager,
    clients_config: &ClientsConfig,
    config: &Config,
    spt_dir: &Path,
    _spt_client: &SptClient,
    current_count: u32,
    desired_count: u32,
) -> Result<()> {
    info!("Scaling up from {current_count} to {desired_count}");

    // 1. Edit fika.jsonc to set amount
    let fika_config_path = spt_dir.join("SPT/user/mods/fika-server/config/fika.jsonc");
    edit_headless_amount(&fika_config_path, desired_count)?;

    // 2. Restart SPT server to pick up new headless count
    info!("Restarting SPT server to register new headless clients");
    // TODO: This should use the server lifecycle manager when available
    // For now, we'll assume the server is already running and the config change will
    // be picked up on next restart (which the supervisor will handle)

    // 3. Wait for new profiles to appear
    // TODO: Implement profile discovery with timeout
    // For now, we'll skip this and assume profiles exist

    // 4. Create containers for new clients
    for i in (current_count + 1)..=desired_count {
        create_client_container(container_mgr, clients_config, config, spt_dir, i).await?;
    }

    Ok(())
}

/// Scale down from current_count to desired_count.
async fn scale_down(
    container_mgr: &ContainerManager,
    _clients_config: &ClientsConfig,
    spt_dir: &Path,
    _spt_client: &SptClient,
    current_count: u32,
    desired_count: u32,
    _force: bool,
) -> Result<()> {
    info!("Scaling down from {current_count} to {desired_count}");

    // TODO: Check for in-raid clients and abort if force=false
    // This requires the SptClient::headless_clients() call to check status

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
    let fika_config_path = spt_dir.join("SPT/user/mods/fika-server/config/fika.jsonc");
    edit_headless_amount(&fika_config_path, desired_count)?;

    // 3. Restart SPT server to deregister removed clients
    info!("Restarting SPT server to deregister removed headless clients");
    // TODO: This should use the server lifecycle manager when available

    Ok(())
}

/// Create a container for a single client.
async fn create_client_container(
    container_mgr: &ContainerManager,
    clients_config: &ClientsConfig,
    config: &Config,
    spt_dir: &Path,
    index: u32,
) -> Result<()> {
    let name = client_container_name(index);
    let overlay_dir = spt_dir.join("clients").join(index.to_string());

    // Set up overlay directory first
    setup_client_overlay(
        spt_dir,
        index,
        &clients_config.install_dir,
        &clients_config.isolated_paths,
    )?;

    // Build volume mounts
    let mut volumes = vec![
        // SPT install directory (read-only)
        VolumeMount {
            host_path: clients_config.install_dir.clone(),
            container_path: "/opt/tarkov".to_string(),
            read_only: true,
            selinux: SelinuxLabel::Private,
        },
    ];

    // Add overlay as read-write mount for isolated paths
    if !clients_config.isolated_paths.is_empty() {
        volumes.push(VolumeMount {
            host_path: overlay_dir,
            container_path: "/opt/tarkov-overlay".to_string(),
            read_only: false,
            selinux: SelinuxLabel::Private,
        });
    }

    // Environment variables
    let mut env = vec![
        ("HEADLESS_INDEX".to_string(), index.to_string()),
        (
            "UDP_PORT".to_string(),
            client_port(clients_config.base_udp_port, index).to_string(),
        ),
    ];

    // Add SPT server host/port if configured
    if let Some(ref host) = config.server_host {
        env.push(("SPT_SERVER_HOST".to_string(), host.clone()));
    }
    if let Some(port) = config.server_port {
        env.push(("SPT_SERVER_PORT".to_string(), port.to_string()));
    }

    // Labels
    let labels = vec![
        (MANAGED_BY_LABEL.to_string(), MANAGED_BY_VALUE.to_string()),
        ("quma.client.index".to_string(), index.to_string()),
    ];

    // Create the container
    let opts = CreateContainerOpts {
        name: name.clone(),
        image: clients_config.image.clone(),
        env,
        volumes,
        ports: vec![], // No published ports for headless clients
        labels,
        user: None,
    };

    let container_id = container_mgr.create_container(opts).await?;
    info!("Created container {name} (id: {container_id})");

    // Start the container
    container_mgr.start(&name).await?;
    info!("Started container {name}");

    Ok(())
}

#[cfg(test)]
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
}
