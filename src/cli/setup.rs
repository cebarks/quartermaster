use std::io::Write;
use std::path::{Path, PathBuf};

use anyhow::{bail, Context, Result};

use crate::config::Config;
use bollard::models::HealthConfig;

use crate::container::{
    ContainerManager, CreateContainerOpts, PortMapping, Protocol, SelinuxLabel, VolumeMount,
    DEFAULT_CONTAINER_NAME, DEFAULT_SPT_PORT, SPT_SERVER_IMAGE,
};
use crate::db::Database;
use crate::spt::detect::{read_spt_version, validate_spt_dir};
use crate::web::auth::hash_password;

use super::common::find_unmanaged_mod_dirs;
use super::Cli;

const DEV_CONTAINER_NAME: &str = "spt-server-dev";

pub struct SetupArgs {
    pub quma_dir: Option<PathBuf>,
    pub path: Option<PathBuf>,
    pub no_fika: bool,
    pub admin_password: Option<String>,
    pub dev: bool,
    pub container_name: Option<String>,
    pub spt_version: Option<String>,
}

impl SetupArgs {
    pub fn effective_dir(&self) -> Option<&Path> {
        self.quma_dir.as_deref().or_else(|| {
            if self.path.is_some() {
                tracing::warn!("--path is deprecated, use --quma-dir instead");
            }
            self.path.as_deref()
        })
    }
}

pub async fn run(args: SetupArgs, cli: &Cli) -> Result<()> {
    println!("=== Quartermaster Setup ===\n");

    // --- Collect input ---
    let data_dir = resolve_data_dir(args.effective_dir())?;
    let install_fika = if args.no_fika { false } else { prompt_fika()? };
    let admin_password = match args.admin_password {
        Some(pw) => {
            if pw.len() < 8 {
                bail!("--admin-password must be at least 8 characters");
            }
            pw
        }
        None => prompt_admin_password()?,
    };
    let container_name = if let Some(name) = args.container_name {
        name
    } else if args.dev {
        DEV_CONTAINER_NAME.to_string()
    } else {
        DEFAULT_CONTAINER_NAME.to_string()
    };

    let params = ResolvedSetup {
        data_dir,
        install_fika,
        admin_password,
        container_name,
        spt_version: args.spt_version,
    };

    // --- Detect path ---
    let mgr = ContainerManager::new(10).context(
        "No container runtime found. Install Podman or Docker and ensure the socket is enabled.",
    )?;

    let dir_state = classify_directory(&params.data_dir)?;

    match dir_state {
        DirState::Empty => bootstrap(&mgr, params, cli).await,
        DirState::ExistingSptNew | DirState::ExistingSptLegacy => {
            wrap_existing(&mgr, params, dir_state, cli).await
        }
    }
}

struct ResolvedSetup {
    data_dir: PathBuf,
    install_fika: bool,
    admin_password: String,
    container_name: String,
    spt_version: Option<String>,
}

#[derive(Debug)]
enum DirState {
    Empty,
    ExistingSptNew,
    ExistingSptLegacy,
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

    // New layout: has spt-server/ subdir with valid SPT install
    if validate_spt_dir(&path.join("spt-server")).is_ok() {
        return Ok(DirState::ExistingSptNew);
    }

    // Legacy layout: SPT markers at root
    if validate_spt_dir(path).is_ok() {
        return Ok(DirState::ExistingSptLegacy);
    }

    bail!(
        "Directory {} exists and contains files but is not a valid Quartermaster or SPT installation.\n\
         Use an empty directory for a fresh setup, or point at an existing install.",
        path.display()
    );
}

fn resolve_data_dir(explicit: Option<&Path>) -> Result<PathBuf> {
    let path = if let Some(p) = explicit {
        p.to_path_buf()
    } else {
        let default = std::env::var_os("HOME")
            .map(|h| PathBuf::from(h).join("spt-server"))
            .unwrap_or_else(|| PathBuf::from("./spt-server"));

        print!(
            "Where should Quartermaster data live? [{}]: ",
            default.display()
        );
        std::io::stdout().flush()?;
        let mut input = String::new();
        std::io::stdin().read_line(&mut input)?;
        let trimmed = input.trim();

        if trimmed.is_empty() {
            default
        } else {
            PathBuf::from(trimmed)
        }
    };

    std::path::absolute(&path)
        .with_context(|| format!("failed to resolve absolute path for {}", path.display()))
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

// TODO(debt): duplicates install_single_mod logic from install.rs — extract a context-free helper
async fn install_from_forge(
    forge: &crate::forge::client::ForgeClient,
    dirs: &crate::dirs::QumaDirs,
    db: &Database,
    config: &Config,
    forge_mod_id: i64,
    label: &str,
) -> Result<()> {
    if let Some(existing) = db.get_mod_by_forge_id(forge_mod_id)? {
        println!(
            "  {} v{} already installed, skipping",
            existing.name, existing.version
        );
        return Ok(());
    }

    let forge_mod = forge
        .get_mod(forge_mod_id, true)
        .await
        .with_context(|| format!("failed to fetch {label} mod info from Forge"))?;

    let version = forge_mod
        .versions
        .as_ref()
        .and_then(|v| v.iter().max_by_key(|v| v.id))
        .ok_or_else(|| anyhow::anyhow!("no versions found for {label} on Forge"))?;

    let download_url = version
        .link
        .as_deref()
        .ok_or_else(|| anyhow::anyhow!("{label} version has no download link"))?;

    let version_str = &version.version;
    println!("  {} v{}", forge_mod.name, version_str);

    super::install::download_and_install(
        forge,
        db,
        dirs,
        config,
        &super::install::ModInstallParams {
            forge_mod_id,
            forge_version_id: version.id,
            download_url,
            name: &forge_mod.name,
            slug: forge_mod.slug.as_deref(),
            version: version_str,
        },
    )
    .await?;

    Ok(())
}

async fn install_infrastructure_from_forge(
    dirs: &crate::dirs::QumaDirs,
    db: &Database,
    config: &Config,
    install_fika: bool,
) -> Result<()> {
    use crate::config::{FIKA_CLIENT_FORGE_ID, FIKA_SERVER_FORGE_ID};
    use crate::forge::client::ForgeClient;

    if !install_fika {
        return Ok(());
    }

    let forge = ForgeClient::new()?;

    println!("\nInstalling Fika...");
    install_from_forge(
        &forge,
        dirs,
        db,
        config,
        FIKA_SERVER_FORGE_ID,
        "Fika Server",
    )
    .await?;
    install_from_forge(
        &forge,
        dirs,
        db,
        config,
        FIKA_CLIENT_FORGE_ID,
        "Fika Client",
    )
    .await?;
    println!("Fika installed.");

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

async fn prompt_spt_version(explicit: Option<&str>) -> Result<crate::spt::releases::SptRelease> {
    use crate::spt::releases;

    if let Some(version) = explicit {
        let releases = releases::list_releases().await?;
        return releases
            .into_iter()
            .find(|r| r.version == version && r.download_url.is_some())
            .ok_or_else(|| {
                anyhow::anyhow!(
                    "SPT version {version} not found or download is no longer available"
                )
            });
    }

    println!("Fetching available SPT versions...");
    let releases = releases::list_releases().await?;
    let available: Vec<_> = releases
        .iter()
        .filter(|r| r.download_url.is_some())
        .collect();

    if available.is_empty() {
        anyhow::bail!("no SPT releases with valid download URLs found");
    }

    println!("\nAvailable SPT versions:");
    for (i, r) in available.iter().enumerate() {
        let marker = if i == 0 { " (latest)" } else { "" };
        println!(
            "  {}. SPT {} (EFT {}){}",
            i + 1,
            r.version,
            r.eft_version,
            marker
        );
    }

    print!("\nSelect version [1]: ");
    std::io::stdout().flush()?;
    let mut input = String::new();
    std::io::stdin().read_line(&mut input)?;
    let trimmed = input.trim();

    let idx = if trimmed.is_empty() {
        0
    } else {
        trimmed
            .parse::<usize>()
            .context("invalid selection")?
            .checked_sub(1)
            .ok_or_else(|| anyhow::anyhow!("selection out of range"))?
    };

    available
        .into_iter()
        .nth(idx)
        .cloned()
        .ok_or_else(|| anyhow::anyhow!("selection out of range"))
}

pub(crate) fn create_container_opts(
    spt_server_dir: &Path,
    container_name: &str,
) -> CreateContainerOpts {
    CreateContainerOpts {
        name: container_name.to_string(),
        image: SPT_SERVER_IMAGE.to_string(),
        env: vec![],
        volumes: vec![VolumeMount {
            host_path: spt_server_dir.to_path_buf(),
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
                "curl -sf http://localhost:6969/launcher/ping || exit 1".to_string(),
            ]),
            interval: Some(30_000_000_000),
            timeout: Some(10_000_000_000),
            retries: Some(3),
            start_period: Some(120_000_000_000),
            start_interval: None,
        }),
        devices: vec![],
        security_opt: vec![],
        cpuset_cpus: None,
        cpuset_mems: None,
    }
}

async fn check_container_name_available(
    mgr: &ContainerManager,
    container_name: &str,
) -> Result<()> {
    match mgr.inspect(container_name).await {
        Ok(_) => bail!(
            "Container '{}' already exists. Remove it with \
             `podman rm {0}` or `docker rm {0}` and re-run setup.",
            container_name
        ),
        Err(_) => Ok(()),
    }
}

fn create_config(dirs: &crate::dirs::QumaDirs, container_name: &str) -> Result<(Config, PathBuf)> {
    let config_path = dirs.config_path();
    let mut config = if config_path.exists() {
        Config::load(&config_path)?
    } else {
        Config::default()
    };
    config.quma_dir = Some(dirs.root.clone());
    config.server_container = Some(container_name.to_string());
    config.server_host = Some("0.0.0.0".to_string());
    config.server_port = Some(DEFAULT_SPT_PORT);
    config.ensure_session_secret();
    config.save(&config_path)?;
    println!("Config saved to {}", config_path.display());
    Ok((config, config_path))
}

fn create_db_and_admin(dirs: &crate::dirs::QumaDirs, admin_password: &str) -> Result<Database> {
    let db_path = dirs.db_path();
    let db = Database::open(&db_path)
        .with_context(|| format!("failed to create database at {}", db_path.display()))?;
    println!("Database initialized at {}", db_path.display());

    if db.has_user_manager()? {
        println!("Admin user already exists.");
    } else {
        let password_hash = hash_password(admin_password)?;
        db.insert_user("admin", None, Some(&password_hash), "admin", false)
            .map_err(|e| anyhow::anyhow!("failed to create admin user: {e}"))?;
        println!("Admin user 'admin' created.");
    }

    Ok(db)
}

fn print_summary(config: &Config, data_dir: &Path, install_fika: bool) {
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

async fn bootstrap(mgr: &ContainerManager, p: ResolvedSetup, _cli: &Cli) -> Result<()> {
    println!("\nNo existing SPT installation found. Bootstrapping from scratch...\n");

    // 1. Create QumaDirs and full directory tree
    let dirs = crate::dirs::QumaDirs::from_root(p.data_dir.clone());
    std::fs::create_dir_all(&dirs.spt_server)
        .with_context(|| format!("failed to create directory {}", dirs.spt_server.display()))?;
    std::fs::create_dir_all(&dirs.headless)
        .with_context(|| format!("failed to create directory {}", dirs.headless.display()))?;
    std::fs::create_dir_all(&dirs.overlay)
        .with_context(|| format!("failed to create directory {}", dirs.overlay.display()))?;
    println!("Created {}", p.data_dir.display());

    // 2. Download and extract SPT server
    let release = prompt_spt_version(p.spt_version.as_deref()).await?;
    println!(
        "\nDownloading SPT {} (EFT {})...",
        release.version, release.eft_version
    );

    let last_log = std::sync::Mutex::new(std::time::Instant::now());
    crate::spt::releases::download_and_extract_release(
        &release,
        &dirs.spt_server,
        |downloaded, total| {
            let mut last = last_log.lock().expect("progress mutex poisoned");
            if last.elapsed().as_secs() >= 5 {
                let dl_mb = downloaded as f64 / 1_048_576.0;
                if let Some(t) = total {
                    let total_mb = t as f64 / 1_048_576.0;
                    println!(
                        "  {dl_mb:.1} / {total_mb:.1} MB ({:.0}%)",
                        dl_mb / total_mb * 100.0
                    );
                } else {
                    println!("  {dl_mb:.1} MB downloaded");
                }
                *last = std::time::Instant::now();
            }
        },
    )
    .await?;
    println!(
        "SPT {} extracted to {}",
        release.version,
        dirs.spt_server.display()
    );

    // 3. Check container name available
    check_container_name_available(mgr, &p.container_name).await?;

    // 4. Pull image
    println!("Pulling {}...", SPT_SERVER_IMAGE);
    mgr.pull_image(SPT_SERVER_IMAGE).await?;
    println!("Image pulled.");

    // 5. Create container (files already on disk — no first-boot needed)
    let opts = create_container_opts(&dirs.spt_server, &p.container_name);
    mgr.create_container(opts).await?;
    println!("Container '{}' created.", p.container_name);

    // 6. Create config
    let (config, _config_path) = create_config(&dirs, &p.container_name)?;

    // 7. Create DB and admin
    let db = create_db_and_admin(&dirs, &p.admin_password)?;

    // 8. Install infrastructure mods from Forge
    install_infrastructure_from_forge(&dirs, &db, &config, p.install_fika).await?;

    // 9. Summary
    print_summary(&config, &p.data_dir, p.install_fika);

    Ok(())
}

// --- Path B: Wrap Existing ---

async fn wrap_existing(
    mgr: &ContainerManager,
    p: ResolvedSetup,
    dir_state: DirState,
    _cli: &Cli,
) -> Result<()> {
    // Create QumaDirs based on layout
    let dirs = match dir_state {
        DirState::ExistingSptNew => crate::dirs::QumaDirs::from_root(p.data_dir.clone()),
        DirState::ExistingSptLegacy => crate::dirs::QumaDirs::from_legacy(p.data_dir.clone()),
        DirState::Empty => unreachable!("Empty state handled by bootstrap"),
    };

    let spt_info = read_spt_version(&dirs.spt_server)?;
    println!(
        "\nExisting SPT {} (EFT {}) detected.\n",
        spt_info.spt_version, spt_info.tarkov_version
    );

    // 1. Detect or create container
    let resolved_container = detect_or_create_container(mgr, &dirs, &p.container_name).await?;

    // 2. Create config
    let (mut config, config_path) = create_config(&dirs, &p.container_name)?;
    config.server_container = Some(resolved_container);
    config.save(&config_path)?;

    // 3. Create DB and admin
    let db = create_db_and_admin(&dirs, &p.admin_password)?;

    // 4. Scan unmanaged mods
    let (unmanaged_dirs, unmanaged_count) = find_unmanaged_mod_dirs(&dirs, &db)?;
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

    // 5. Install infrastructure mods from Forge
    install_infrastructure_from_forge(&dirs, &db, &config, p.install_fika).await?;

    // 6. Summary
    print_summary(&config, &p.data_dir, p.install_fika);

    Ok(())
}

async fn detect_or_create_container(
    mgr: &ContainerManager,
    dirs: &crate::dirs::QumaDirs,
    container_name: &str,
) -> Result<String> {
    let detected = mgr.detect_spt_containers(dirs).await?;

    if detected.len() == 1 {
        println!("Detected existing container: {}", detected[0]);
        return Ok(detected[0].clone());
    }

    if detected.len() > 1 {
        let managed = mgr.detect_containers_by_label("managed-by", "quma").await?;
        let quma_managed: Vec<_> = detected
            .iter()
            .filter(|name| managed.contains(name))
            .collect();
        if let Some(preferred) = quma_managed.first() {
            println!(
                "Multiple containers detected, preferring quma-managed: {}",
                preferred
            );
            return Ok((*preferred).clone());
        }
        println!("Multiple containers detected, using first: {}", detected[0]);
        return Ok(detected[0].clone());
    }

    // No container found — create one
    println!("No existing container found. Creating one...");
    check_container_name_available(mgr, container_name).await?;

    println!("Pulling {}...", SPT_SERVER_IMAGE);
    mgr.pull_image(SPT_SERVER_IMAGE).await?;

    let opts = create_container_opts(&dirs.spt_server, container_name);
    mgr.create_container(opts).await?;
    println!("Container '{}' created.", container_name);

    Ok(container_name.to_string())
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
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
    fn classify_valid_spt_dir_legacy() {
        let tmp = tempfile::tempdir().unwrap();
        let spt_dir = tmp.path();

        // Create minimum SPT structure at root (legacy layout)
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
        assert!(matches!(state, DirState::ExistingSptLegacy));
    }

    #[test]
    fn classify_new_layout_with_spt_server_subdir() {
        let tmp = tempfile::tempdir().unwrap();
        let quma_root = tmp.path();
        let spt_dir = quma_root.join("spt-server");

        // Create minimum SPT structure in spt-server/ subdir (new layout)
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

        let state = classify_directory(quma_root).unwrap();
        assert!(matches!(state, DirState::ExistingSptNew));
    }

    #[test]
    fn setup_args_effective_dir_prefers_quma_dir() {
        let args = SetupArgs {
            quma_dir: Some(PathBuf::from("/quma")),
            path: Some(PathBuf::from("/path")),
            no_fika: false,
            admin_password: None,
            dev: false,
            container_name: None,
            spt_version: None,
        };
        assert_eq!(args.effective_dir(), Some(Path::new("/quma")));
    }

    #[test]
    fn setup_args_effective_dir_falls_back_to_path() {
        let args = SetupArgs {
            quma_dir: None,
            path: Some(PathBuf::from("/path")),
            no_fika: false,
            admin_password: None,
            dev: false,
            container_name: None,
            spt_version: None,
        };
        assert_eq!(args.effective_dir(), Some(Path::new("/path")));
    }

    #[test]
    fn setup_args_effective_dir_none_when_both_none() {
        let args = SetupArgs {
            quma_dir: None,
            path: None,
            no_fika: false,
            admin_password: None,
            dev: false,
            container_name: None,
            spt_version: None,
        };
        assert_eq!(args.effective_dir(), None);
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
            .contains("not a valid Quartermaster or SPT installation"));
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
    fn create_container_opts_defaults() {
        let dir = PathBuf::from("/data/spt");
        let opts = create_container_opts(&dir, DEFAULT_CONTAINER_NAME);
        assert_eq!(opts.name, "spt-server");
        assert_eq!(opts.image, SPT_SERVER_IMAGE);
        assert!(
            opts.env.is_empty(),
            "no env vars needed for purpose-built image"
        );
        assert_eq!(opts.volumes[0].container_path, "/opt/server");
        assert!(opts.healthcheck.is_some());
    }

    #[test]
    fn create_container_opts_dev_name() {
        let dir = PathBuf::from("/data/spt");
        let opts = create_container_opts(&dir, DEV_CONTAINER_NAME);
        assert_eq!(opts.name, "spt-server-dev");
    }
}
