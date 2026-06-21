use std::io::Write;
use std::path::{Path, PathBuf};

use anyhow::{bail, Context, Result};

use crate::config::Config;
use bollard::models::HealthConfig;

use crate::container::{
    ContainerManager, CreateContainerOpts, PortMapping, Protocol, SelinuxLabel, VolumeMount,
    DEFAULT_CONTAINER_NAME, DEFAULT_SPT_PORT, SPT_SERVER_IMAGE,
};
use crate::db::users::Role;
use crate::db::Database;
use crate::spt::detect::{read_spt_version, validate_spt_dir};
use crate::web::auth::hash_password;

use super::common::find_unmanaged_mod_dirs;
use super::Cli;

pub async fn run(path: Option<PathBuf>, no_fika: bool, no_modsync: bool, cli: &Cli) -> Result<()> {
    println!("=== Quartermaster Setup ===\n");

    // --- Collect input ---
    let data_dir = resolve_data_dir(path)?;
    let install_fika = if no_fika { false } else { prompt_fika()? };
    let install_modsync = if no_modsync { false } else { prompt_modsync()? };
    let admin_password = prompt_admin_password()?;
    let forge_token = prompt_forge_token()?;

    // --- Detect path ---
    let mgr = ContainerManager::new().context(
        "No container runtime found. Install Podman or Docker and ensure the socket is enabled.",
    )?;

    let dir_state = classify_directory(&data_dir)?;

    match dir_state {
        DirState::Empty => {
            bootstrap(
                &mgr,
                &data_dir,
                install_fika,
                install_modsync,
                &admin_password,
                forge_token,
                cli,
            )
            .await
        }
        DirState::ExistingSpt => {
            wrap_existing(
                &mgr,
                &data_dir,
                install_fika,
                install_modsync,
                &admin_password,
                forge_token,
                cli,
            )
            .await
        }
    }
}

#[derive(Debug)]
enum DirState {
    Empty,
    ExistingSpt,
}

fn classify_directory(path: &Path) -> Result<DirState> {
    if !path.exists() {
        return Ok(DirState::Empty);
    }

    if path.is_file() {
        bail!("{} is a file, not a directory.", path.display());
    }

    // Check if empty
    let mut entries = std::fs::read_dir(path)
        .with_context(|| format!("failed to read directory {}", path.display()))?;

    if entries.next().is_none() {
        return Ok(DirState::Empty);
    }

    // Non-empty — check if it's a valid SPT install
    if validate_spt_dir(path).is_ok() {
        return Ok(DirState::ExistingSpt);
    }

    bail!(
        "Directory {} exists and contains files but is not a valid SPT installation.\n\
         Use an empty directory for a fresh setup, or point at an existing SPT install.",
        path.display()
    );
}

fn resolve_data_dir(explicit: Option<PathBuf>) -> Result<PathBuf> {
    if let Some(p) = explicit {
        return Ok(p);
    }

    let default = dirs::home_dir()
        .map(|h| h.join("spt-server"))
        .unwrap_or_else(|| PathBuf::from("./spt-server"));

    print!("Where should server data live? [{}]: ", default.display());
    std::io::stdout().flush()?;
    let mut input = String::new();
    std::io::stdin().read_line(&mut input)?;
    let trimmed = input.trim();

    if trimmed.is_empty() {
        Ok(default)
    } else {
        Ok(PathBuf::from(trimmed))
    }
}

fn prompt_fika() -> Result<bool> {
    print!("Install Fika for multiplayer? [Y/n]: ");
    std::io::stdout().flush()?;
    let mut input = String::new();
    std::io::stdin().read_line(&mut input)?;
    let trimmed = input.trim();

    Ok(trimmed.is_empty()
        || trimmed.eq_ignore_ascii_case("y")
        || trimmed.eq_ignore_ascii_case("yes"))
}

fn prompt_modsync() -> Result<bool> {
    print!("Install NarcoNet for client mod syncing? [Y/n]: ");
    std::io::stdout().flush()?;
    let mut input = String::new();
    std::io::stdin().read_line(&mut input)?;
    let trimmed = input.trim();

    Ok(trimmed.is_empty()
        || trimmed.eq_ignore_ascii_case("y")
        || trimmed.eq_ignore_ascii_case("yes"))
}

async fn install_narconet_from_forge(
    spt_dir: &Path,
    db: &Database,
    config: &Config,
    forge_token: Option<String>,
) -> Result<()> {
    use crate::config::NARCONET_FORGE_MOD_ID;
    use crate::forge::client::ForgeClient;

    println!("\nInstalling NarcoNet...");

    let forge = ForgeClient::new(forge_token)?;
    let forge_mod = forge
        .get_mod(NARCONET_FORGE_MOD_ID, true)
        .await
        .context("failed to fetch NarcoNet mod info from Forge")?;

    let version = forge_mod
        .versions
        .as_ref()
        .and_then(|v| v.first())
        .ok_or_else(|| anyhow::anyhow!("no versions found for NarcoNet on Forge"))?;

    let download_url = version
        .link
        .as_deref()
        .ok_or_else(|| anyhow::anyhow!("NarcoNet version has no download link"))?;

    let version_str = &version.version;
    let version_id = version.id;
    println!("Downloading NarcoNet v{version_str}...");

    let tmp_dir = tempfile::tempdir().context("failed to create temp directory")?;
    let archive_path = tmp_dir.path().join("narconet.zip");
    forge
        .download_file(download_url, &archive_path)
        .await
        .context("failed to download NarcoNet")?;

    println!("Extracting...");
    // TODO(debt): consider extracting shared install logic with install_single_mod
    let db_id = crate::ops::install_mod_from_archive(
        db,
        spt_dir,
        config,
        NARCONET_FORGE_MOD_ID,
        version_id,
        &forge_mod.name,
        forge_mod.slug.as_deref(),
        version_str,
        &archive_path,
    )?;

    let file_count = db.get_files_for_mod(db_id)?.len();
    println!("NarcoNet v{version_str} installed ({file_count} files).");
    Ok(())
}

fn prompt_admin_password() -> Result<String> {
    loop {
        let password = rpassword::prompt_password("Admin password (min 8 chars): ")
            .context("failed to read password")?;

        if password.len() < 8 {
            println!("Password must be at least 8 characters. Try again.");
            continue;
        }

        let confirm = rpassword::prompt_password("Confirm password: ")
            .context("failed to read password confirmation")?;

        if password != confirm {
            println!("Passwords do not match. Try again.");
            continue;
        }

        return Ok(password);
    }
}

fn prompt_forge_token() -> Result<Option<String>> {
    println!("\nA Forge API token increases rate limits for mod downloads.");
    println!("Get one at: https://forge.sp-tarkov.com/account/settings");
    print!("Forge API token (leave blank to skip): ");
    std::io::stdout().flush()?;
    let mut input = String::new();
    std::io::stdin().read_line(&mut input)?;
    let trimmed = input.trim();

    if trimmed.is_empty() {
        Ok(None)
    } else {
        Ok(Some(trimmed.to_string()))
    }
}

fn create_container_opts(data_dir: &Path, install_fika: bool) -> CreateContainerOpts {
    let fika_mode = if install_fika { "install" } else { "disabled" };

    CreateContainerOpts {
        name: DEFAULT_CONTAINER_NAME.to_string(),
        image: SPT_SERVER_IMAGE.to_string(),
        env: vec![
            ("LISTEN_ALL_NETWORKS".to_string(), "true".to_string()),
            ("FIKA_MODE".to_string(), fika_mode.to_string()),
        ],
        volumes: vec![VolumeMount {
            host_path: data_dir.to_path_buf(),
            container_path: "/opt/server".to_string(),
            read_only: false,
            selinux: SelinuxLabel::Private,
        }],
        ports: vec![PortMapping {
            host_port: DEFAULT_SPT_PORT,
            container_port: DEFAULT_SPT_PORT,
            protocol: Protocol::Tcp,
        }],
        labels: vec![("managed-by".to_string(), "quma".to_string())],
        user: None,
        healthcheck: Some(HealthConfig {
            test: Some(vec![
                "CMD-SHELL".to_string(),
                "wget -q --spider http://localhost:6969/launcher/ping || exit 1".to_string(),
            ]),
            interval: Some(30_000_000_000), // 30s in nanoseconds
            timeout: Some(10_000_000_000),  // 10s
            retries: Some(3),
            start_period: Some(120_000_000_000), // 120s - SPT server takes a while to boot
            start_interval: None,
        }),
    }
}

async fn check_container_name_available(mgr: &ContainerManager) -> Result<()> {
    match mgr.inspect(DEFAULT_CONTAINER_NAME).await {
        Ok(_) => bail!(
            "Container '{}' already exists. Remove it with \
             `podman rm {0}` or `docker rm {0}` and re-run setup.",
            DEFAULT_CONTAINER_NAME
        ),
        Err(_) => Ok(()),
    }
}

async fn wait_for_server(config: &Config, spt_dir: &Path) -> Result<()> {
    let (host, port) = crate::server_detect::resolve_server_addr(config, spt_dir);
    let spt_client = crate::spt::server::SptClient::new(&host, port)?;

    println!("Waiting for server to start (timeout: 180s)...");
    let start_time = std::time::Instant::now();
    let timeout = std::time::Duration::from_secs(180);

    loop {
        if start_time.elapsed() > timeout {
            bail!(
                "Server did not respond within 180s. Check container logs with \
                 `podman logs {}` or `docker logs {0}`.",
                DEFAULT_CONTAINER_NAME
            );
        }

        match spt_client.ping().await {
            Ok(ping) if ping.ok => {
                println!("Server is ready (responded in {}ms).", ping.latency_ms);
                return Ok(());
            }
            _ => {
                // Connection refused or not ready yet — keep waiting
                print!(".");
                std::io::stdout().flush()?;
                tokio::time::sleep(std::time::Duration::from_secs(3)).await;
            }
        }
    }
}

fn create_config(
    data_dir: &Path,
    forge_token: Option<String>,
    cli: &Cli,
) -> Result<(Config, PathBuf)> {
    let config_path = Config::resolve_path(cli.config.as_deref(), Some(data_dir));
    let mut config = if config_path.exists() {
        Config::load(&config_path)?
    } else {
        Config::default()
    };
    config.spt_dir = Some(data_dir.to_path_buf());
    config.server_container = Some(DEFAULT_CONTAINER_NAME.to_string());
    config.server_host = Some("0.0.0.0".to_string());
    config.server_port = Some(DEFAULT_SPT_PORT);
    config.forge_token = forge_token;
    config.ensure_session_secret();
    config.save(&config_path)?;
    println!("Config saved to {}", config_path.display());
    Ok((config, config_path))
}

fn create_db_and_admin(data_dir: &Path, admin_password: &str) -> Result<Database> {
    let db_path = data_dir.join("quartermaster.db");
    let db = Database::open(&db_path)
        .with_context(|| format!("failed to create database at {}", db_path.display()))?;
    println!("Database initialized at {}", db_path.display());

    if db.admin_exists()? {
        println!("Admin user already exists.");
    } else {
        let password_hash = hash_password(admin_password)?;
        db.insert_user("admin", None, Some(&password_hash), Role::Admin)
            .map_err(|e| anyhow::anyhow!("failed to create admin user: {e}"))?;
        println!("Admin user 'admin' created.");
    }

    Ok(db)
}

fn print_summary(
    config: &Config,
    data_dir: &Path,
    install_fika: bool,
    install_modsync: bool,
    forge_token_set: bool,
) {
    println!("\n=== Setup Complete ===\n");
    println!("SPT directory: {}", data_dir.display());
    if let Some(ref container) = config.server_container {
        println!("Container: {container}");
    }
    println!(
        "Fika: {}",
        if install_fika {
            "installed"
        } else {
            "disabled"
        }
    );
    println!(
        "NarcoNet: {}",
        if install_modsync {
            "installed"
        } else {
            "skipped"
        }
    );
    println!(
        "Forge API token: {}",
        if forge_token_set {
            "configured"
        } else {
            "not set (use QUMA_FORGE_TOKEN or edit config)"
        }
    );
    println!("Web UI: http://{}:{}", config.web_bind, config.web_port);
    println!("Admin user: admin");
    println!("\nNext steps:");
    println!("  quma serve              Start the web UI");
    println!("  quma server start       Start the SPT server");
    println!("  quma invite             Generate invite codes for players");
    println!("\nNetwork requirements (for multiplayer):");
    println!("  TCP 6969 inbound        SPT server");
    println!("  UDP 25565 inbound       Fika P2P raids (whoever hosts)");
    println!("  Consider UPnP or VPN as alternatives to port forwarding.");
}

// --- Path A: Bootstrap ---

async fn bootstrap(
    mgr: &ContainerManager,
    data_dir: &Path,
    install_fika: bool,
    install_modsync: bool,
    admin_password: &str,
    forge_token: Option<String>,
    cli: &Cli,
) -> Result<()> {
    println!("\nNo existing SPT installation found. Bootstrapping from scratch...\n");

    // 1. Create data directory
    std::fs::create_dir_all(data_dir)
        .with_context(|| format!("failed to create directory {}", data_dir.display()))?;
    println!("Created {}", data_dir.display());

    // 2. Check container name available
    check_container_name_available(mgr).await?;

    // 3. Pull image
    println!("Pulling {}...", SPT_SERVER_IMAGE);
    mgr.pull_image(SPT_SERVER_IMAGE).await?;
    println!("Image pulled.");

    // 4. Create container
    let opts = create_container_opts(data_dir, install_fika);
    mgr.create_container(opts).await?;
    println!("Container '{}' created.", DEFAULT_CONTAINER_NAME);

    // 5. First boot
    println!("\nStarting first boot...");
    mgr.start(DEFAULT_CONTAINER_NAME).await?;

    // 6. Create config (needed for wait_for_server to resolve address)
    let forge_token_set = forge_token.is_some();
    let (config, _config_path) = create_config(data_dir, forge_token, cli)?;

    // 7. Wait for server
    wait_for_server(&config, data_dir).await?;

    // 8. Stop server
    println!("\nStopping server after first boot...");
    mgr.stop(DEFAULT_CONTAINER_NAME).await?;
    println!("Server stopped.");

    // 9. Create DB and admin
    let db = create_db_and_admin(data_dir, admin_password)?;

    // 10. Install NarcoNet
    if install_modsync {
        install_narconet_from_forge(data_dir, &db, &config, config.forge_token.clone()).await?;
    }

    // 11. Summary
    print_summary(
        &config,
        data_dir,
        install_fika,
        install_modsync,
        forge_token_set,
    );

    Ok(())
}

// --- Path B: Wrap Existing ---

async fn wrap_existing(
    mgr: &ContainerManager,
    data_dir: &Path,
    install_fika: bool,
    install_modsync: bool,
    admin_password: &str,
    forge_token: Option<String>,
    cli: &Cli,
) -> Result<()> {
    let spt_info = read_spt_version(data_dir)?;
    println!(
        "\nExisting SPT {} (EFT {}) detected.\n",
        spt_info.spt_version, spt_info.tarkov_version
    );

    // 1. Detect or create container
    let container_name = detect_or_create_container(mgr, data_dir, install_fika).await?;

    // 2. Create config
    let forge_token_set = forge_token.is_some();
    let (mut config, config_path) = create_config(data_dir, forge_token, cli)?;
    config.server_container = Some(container_name);
    config.save(&config_path)?;

    // 3. Create DB and admin
    let db = create_db_and_admin(data_dir, admin_password)?;

    // 4. Scan unmanaged mods
    let (unmanaged_dirs, unmanaged_count) = find_unmanaged_mod_dirs(data_dir, &db)?;
    if unmanaged_dirs.is_empty() {
        println!("No unmanaged mod files found.");
    } else {
        println!(
            "Found {} unmanaged mod director{} ({} files).",
            unmanaged_dirs.len(),
            if unmanaged_dirs.len() == 1 {
                "y"
            } else {
                "ies"
            },
            unmanaged_count
        );
        for dir in unmanaged_dirs.keys() {
            println!("  {}", dir);
        }
        println!("\nManage them through the web UI or reinstall via Forge.");
    }

    // 5. Install NarcoNet
    if install_modsync {
        install_narconet_from_forge(data_dir, &db, &config, config.forge_token.clone()).await?;
    }

    // 6. Summary
    print_summary(
        &config,
        data_dir,
        install_fika,
        install_modsync,
        forge_token_set,
    );

    Ok(())
}

async fn detect_or_create_container(
    mgr: &ContainerManager,
    data_dir: &Path,
    install_fika: bool,
) -> Result<String> {
    let detected = mgr.detect_spt_containers(data_dir).await?;

    if detected.len() == 1 {
        println!("Detected existing container: {}", detected[0]);
        return Ok(detected[0].clone());
    }

    if detected.len() > 1 {
        // TODO(debt): use managed-by=quma label to prefer quma-managed containers
        println!("Multiple containers detected, using first: {}", detected[0]);
        return Ok(detected[0].clone());
    }

    // No container found — create one
    println!("No existing container found. Creating one...");
    check_container_name_available(mgr).await?;

    println!("Pulling {}...", SPT_SERVER_IMAGE);
    mgr.pull_image(SPT_SERVER_IMAGE).await?;

    let opts = create_container_opts(data_dir, install_fika);
    mgr.create_container(opts).await?;
    println!("Container '{}' created.", DEFAULT_CONTAINER_NAME);

    Ok(DEFAULT_CONTAINER_NAME.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn classify_empty_dir() {
        let tmp = tempfile::tempdir().unwrap();
        let state = classify_directory(tmp.path()).unwrap();
        assert!(matches!(state, DirState::Empty));
    }

    #[test]
    fn classify_nonexistent_dir() {
        let tmp = tempfile::tempdir().unwrap();
        let nonexistent = tmp.path().join("does-not-exist");
        let state = classify_directory(&nonexistent).unwrap();
        assert!(matches!(state, DirState::Empty));
    }

    #[test]
    fn classify_valid_spt_dir() {
        let tmp = tempfile::tempdir().unwrap();
        let spt_dir = tmp.path();

        // Create minimum SPT structure
        std::fs::create_dir_all(spt_dir.join("SPT")).unwrap();
        std::fs::write(spt_dir.join("SPT/SPT.Server.exe"), b"").unwrap();
        let configs_dir = spt_dir.join("SPT/SPT_Data/configs");
        std::fs::create_dir_all(&configs_dir).unwrap();
        std::fs::write(
            configs_dir.join("core.json"),
            r#"{"compatibleTarkovVersion": "0.16.9-40087"}"#,
        )
        .unwrap();
        std::fs::create_dir_all(spt_dir.join("SPT/user/mods")).unwrap();
        std::fs::create_dir_all(spt_dir.join("BepInEx/plugins")).unwrap();

        let state = classify_directory(spt_dir).unwrap();
        assert!(matches!(state, DirState::ExistingSpt));
    }

    #[test]
    fn classify_non_spt_nonempty_dir_fails() {
        let tmp = tempfile::tempdir().unwrap();
        std::fs::write(tmp.path().join("random.txt"), b"hello").unwrap();
        let result = classify_directory(tmp.path());
        assert!(result.is_err());
        assert!(result
            .unwrap_err()
            .to_string()
            .contains("not a valid SPT installation"));
    }

    #[test]
    fn classify_file_path_fails() {
        let tmp = tempfile::tempdir().unwrap();
        let file = tmp.path().join("afile");
        std::fs::write(&file, b"data").unwrap();
        let result = classify_directory(&file);
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("is a file"));
    }

    #[test]
    fn create_container_opts_fika_enabled() {
        let dir = PathBuf::from("/data/spt");
        let opts = create_container_opts(&dir, true);
        assert!(opts
            .env
            .iter()
            .any(|(k, v)| k == "FIKA_MODE" && v == "install"));
        assert_eq!(opts.name, "spt-server");
        assert_eq!(opts.volumes[0].container_path, "/opt/server");
    }

    #[test]
    fn create_container_opts_fika_disabled() {
        let dir = PathBuf::from("/data/spt");
        let opts = create_container_opts(&dir, false);
        assert!(opts
            .env
            .iter()
            .any(|(k, v)| k == "FIKA_MODE" && v == "disabled"));
    }
}
