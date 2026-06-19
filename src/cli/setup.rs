use std::io::Write;
use std::path::{Path, PathBuf};

use anyhow::{bail, Context, Result};

use crate::config::Config;
use crate::db::users::Role;
use crate::db::Database;
use crate::forge::client::ForgeClient;
use crate::podman::PodmanClient;
use crate::spt::detect::{detect_spt_dir, read_spt_version, validate_spt_dir};
use crate::spt::profiles::list_profiles;
use crate::web::auth::hash_password;

use super::common::{confirm, find_unmanaged_mod_dirs};
use super::Cli;

const FIKA_FORGE_MOD_ID: i64 = 2326;

pub async fn run(non_interactive: bool, skip_fika: bool, cli: &Cli) -> Result<()> {
    println!("=== Quartermaster Setup ===\n");

    // Step 1: Detect/confirm SPT directory
    let spt_dir = detect_spt_directory(cli, non_interactive)?;

    // Step 2: Validate SPT install
    let spt_info = read_spt_version(&spt_dir)?;
    println!(
        "SPT {} (EFT {}) detected at {}",
        spt_info.spt_version,
        spt_info.tarkov_version,
        spt_dir.display()
    );

    // Step 3: Create config
    let config_path = Config::resolve_path(cli.config.as_deref(), Some(&spt_dir));
    let mut config = if config_path.exists() {
        Config::load(&config_path)?
    } else {
        Config::default()
    };
    config.spt_dir = Some(spt_dir.clone());
    config.ensure_session_secret();

    // Step 4: Configure Podman container
    configure_container(&spt_dir, &mut config, non_interactive).await?;

    // Step 5: Configure networking
    configure_networking(&spt_dir, &mut config, non_interactive)?;

    // Save config so far (container + networking)
    config.save(&config_path)?;
    println!("\nConfig saved to {}", config_path.display());

    // Step 6: Create database and build CliContext for reuse
    let db_path = spt_dir.join("quartermaster.db");
    let db = Database::open(&db_path)
        .with_context(|| format!("failed to create database at {}", db_path.display()))?;
    println!("Database initialized at {}", db_path.display());

    let forge = ForgeClient::new(config.forge_token.clone())?;
    let ctx = super::common::CliContext {
        spt_dir: spt_dir.clone(),
        spt_info: spt_info.clone(),
        config: config.clone(),
        db,
        forge,
    };

    // Step 7: Install Fika (if not skipped)
    if !skip_fika {
        install_fika(&ctx).await?;
    } else {
        println!("\nSkipping Fika installation (--skip-fika).");
    }

    // Step 8: First boot (if container configured)
    if config.server_container.is_some() && !skip_fika {
        first_boot(&config, &spt_dir, non_interactive).await?;
    }

    // Step 9: Configure fika.jsonc (if Fika installed and first boot ran)
    if config.server_container.is_some() && !skip_fika {
        configure_fika(&spt_dir, non_interactive)?;
    }

    // Step 10: Scan for unmanaged mods (same as quma init)
    scan_unmanaged(&spt_dir, &ctx.db)?;

    // Step 11: Create admin user
    create_admin_user(&spt_dir, &ctx.db, non_interactive)?;

    // Step 12: Print summary
    print_summary(&config, &spt_dir, skip_fika);

    Ok(())
}

fn detect_spt_directory(cli: &Cli, non_interactive: bool) -> Result<PathBuf> {
    match detect_spt_dir(cli.spt_dir.as_deref(), None) {
        Ok(dir) => {
            println!("Found SPT directory: {}", dir.display());
            if !non_interactive && !confirm("Use this directory?")? {
                bail!("Setup cancelled. Use --spt-dir to specify the SPT directory.");
            }
            Ok(dir)
        }
        Err(_) => {
            if non_interactive {
                bail!("Could not auto-detect SPT directory. Use --spt-dir to specify it.");
            }
            print!("Enter SPT server directory path: ");
            std::io::stdout().flush()?;
            let mut input = String::new();
            std::io::stdin().read_line(&mut input)?;
            let path = PathBuf::from(input.trim());
            validate_spt_dir(&path)?;
            Ok(path)
        }
    }
}

async fn configure_container(
    spt_dir: &Path,
    config: &mut Config,
    non_interactive: bool,
) -> Result<()> {
    println!("\n--- Container Configuration ---");

    if let Some(name) = config.server_container.as_deref() {
        println!("Container already configured: {name}");
        return Ok(());
    }

    let detected = PodmanClient::detect_spt_containers(spt_dir).await?;

    if detected.len() == 1 {
        let name = &detected[0];
        println!("Detected Podman container: {name}");
        if non_interactive || confirm("Use this container?")? {
            config.server_container = Some(name.clone());
            return Ok(());
        }
    } else if detected.len() > 1 {
        println!("Multiple containers detected:");
        for (i, name) in detected.iter().enumerate() {
            println!("  [{}] {}", i + 1, name);
        }
        if !non_interactive {
            print!("Select [1-{}] or press Enter to skip: ", detected.len());
            std::io::stdout().flush()?;
            let mut input = String::new();
            std::io::stdin().read_line(&mut input)?;
            let input = input.trim();
            if !input.is_empty() {
                if let Ok(choice) = input.parse::<usize>() {
                    if choice >= 1 && choice <= detected.len() {
                        config.server_container = Some(detected[choice - 1].clone());
                        return Ok(());
                    }
                }
            }
        }
    } else {
        println!("No Podman containers detected.");
    }

    if !non_interactive {
        print!("Enter container name (or press Enter to skip): ");
        std::io::stdout().flush()?;
        let mut input = String::new();
        std::io::stdin().read_line(&mut input)?;
        let name = input.trim();
        if !name.is_empty() {
            config.server_container = Some(name.to_string());
        } else {
            println!(
                "Skipping container setup. Set it later with: quma config set server_container <name>"
            );
        }
    }

    Ok(())
}

/// Read and optionally update SPT's http.json networking config.
fn configure_networking(spt_dir: &Path, config: &mut Config, non_interactive: bool) -> Result<()> {
    println!("\n--- Network Configuration ---");

    let http_json_path = spt_dir.join("SPT/SPT_Data/configs/http.json");

    if http_json_path.exists() {
        let contents = std::fs::read_to_string(&http_json_path)?;
        let mut json: serde_json::Value = serde_json::from_str(&contents)
            .with_context(|| format!("failed to parse {}", http_json_path.display()))?;

        let current_ip = json
            .get("ip")
            .and_then(|v| v.as_str())
            .unwrap_or("127.0.0.1");
        let current_port = json.get("port").and_then(|v| v.as_u64()).unwrap_or(6969) as u16;

        println!("Current SPT server bind: {current_ip}:{current_port}");

        if current_ip == "127.0.0.1" {
            println!("SPT is bound to localhost — remote players won't be able to connect.");
            let should_update =
                non_interactive || confirm("Set bind to 0.0.0.0 (all interfaces)?")?;
            if should_update {
                json["ip"] = serde_json::Value::String("0.0.0.0".to_string());
                let updated = serde_json::to_string_pretty(&json)?;
                std::fs::write(&http_json_path, updated)?;
                println!("Updated http.json: ip = 0.0.0.0");
                println!(
                    "WARNING: SPT server will now listen on all network interfaces.\n\
                     Ensure your firewall allows TCP port {} only from trusted networks.\n\
                     Without firewall rules, the server is accessible from the public internet.",
                    current_port
                );
            }
        }

        config.server_host = Some(
            json.get("ip")
                .and_then(|v| v.as_str())
                .unwrap_or("0.0.0.0")
                .to_string(),
        );
        config.server_port = Some(current_port);
    } else {
        println!("http.json not found — using defaults (127.0.0.1:6969)");
        config.server_host = Some("127.0.0.1".to_string());
        config.server_port = Some(6969);
    }

    Ok(())
}

async fn install_fika(ctx: &super::common::CliContext) -> Result<()> {
    println!("\n--- Fika Installation ---");

    if ctx.db.get_mod_by_forge_id(FIKA_FORGE_MOD_ID)?.is_some() {
        println!("Fika is already installed.");
        return Ok(());
    }

    println!("Looking up Fika on Forge...");
    let versions = ctx
        .forge
        .get_versions(FIKA_FORGE_MOD_ID, Some(&ctx.spt_info.spt_version))
        .await?;

    let version = match versions.into_iter().next() {
        Some(v) => v,
        None => {
            println!(
                "Warning: no Fika version compatible with SPT {} found. Skipping Fika install.",
                ctx.spt_info.spt_version
            );
            return Ok(());
        }
    };

    println!("Installing Fika v{}...", version.version);
    crate::cli::install::install_with_deps(ctx, FIKA_FORGE_MOD_ID, version.id).await?;

    println!("Fika installed successfully.");
    Ok(())
}

async fn first_boot(config: &Config, spt_dir: &Path, non_interactive: bool) -> Result<()> {
    println!("\n--- First Boot ---");

    let container = match config.server_container.as_deref() {
        Some(c) => c,
        None => bail!("no server container configured"),
    };
    let podman = PodmanClient::new(container);

    let running = match podman.is_running().await {
        Ok(r) => r,
        Err(e) => {
            eprintln!("Warning: could not check if server is running: {e:#}");
            false
        }
    };
    if running {
        println!("Server is already running.");
        return Ok(());
    }

    if !non_interactive && !confirm("Start SPT server for first boot (generates fika.jsonc)?")? {
        println!("Skipping first boot. Start the server manually to generate config files.");
        return Ok(());
    }

    println!("Starting SPT server...");
    podman.start().await?;

    let (host, port) = crate::server_detect::resolve_server_addr(config, spt_dir);
    let spt_client = crate::spt::server::SptClient::new(&host, port)?;

    println!("Waiting for server to start (timeout: 90s)...");
    let start_time = std::time::Instant::now();
    let timeout = std::time::Duration::from_secs(90);

    loop {
        if start_time.elapsed() > timeout {
            println!("Server did not respond within 90s. Check `quma server logs` for errors.");
            println!("You may need to start and configure it manually.");
            return Ok(());
        }

        let ping = spt_client.ping().await?;
        if ping.ok {
            println!("Server is ready (responded in {}ms).", ping.latency_ms);
            break;
        }

        tokio::time::sleep(std::time::Duration::from_secs(3)).await;
    }

    // Stop the server after first boot
    println!("Stopping server after first boot...");
    podman.stop().await?;
    println!("Server stopped.");

    Ok(())
}

/// Replace a boolean value in raw JSONC text, preserving comments and formatting.
fn replace_json_bool(raw: &str, key: &str, value: bool) -> String {
    let pattern = format!("\"{}\"", key);
    let mut result = String::with_capacity(raw.len());
    let remaining = raw;

    if let Some(key_pos) = remaining.find(&pattern) {
        let after_key = key_pos + pattern.len();
        if let Some(colon_offset) = remaining[after_key..].find(':') {
            let after_colon = after_key + colon_offset + 1;
            let rest = &remaining[after_colon..];
            let trimmed = rest.trim_start();
            let ws_len = rest.len() - trimmed.len();
            let old_val = if trimmed.starts_with("true") {
                "true"
            } else {
                "false"
            };
            result.push_str(&remaining[..after_colon + ws_len]);
            result.push_str(if value { "true" } else { "false" });
            result.push_str(&remaining[after_colon + ws_len + old_val.len()..]);
            return result;
        }
    }

    remaining.to_string()
}

/// Configure key fika.jsonc settings after first boot generates the file.
fn configure_fika(spt_dir: &Path, non_interactive: bool) -> Result<()> {
    println!("\n--- Fika Configuration ---");

    let fika_config_path = spt_dir.join("SPT/user/mods/fika-server/assets/configs/fika.jsonc");
    if !fika_config_path.exists() {
        println!("fika.jsonc not found — Fika may not have generated its config yet.");
        println!("Start the server manually, then edit fika.jsonc.");
        return Ok(());
    }

    let raw = std::fs::read_to_string(&fika_config_path)
        .with_context(|| format!("failed to read {}", fika_config_path.display()))?;
    let stripped = json_comments::StripComments::new(raw.as_bytes());
    let json: serde_json::Value =
        serde_json::from_reader(stripped).with_context(|| "failed to parse fika.jsonc")?;

    if non_interactive {
        println!("Using Fika defaults (non-interactive mode).");
        return Ok(());
    }

    println!("Configure Fika settings (press Enter to keep default):\n");

    let settings = [
        ("friendlyFire", "Friendly fire", true),
        ("forceSaveOnDeath", "Force save on death", true),
        ("sharedQuestProgression", "Shared quest progression", false),
    ];

    let mut updated_raw = raw.clone();
    let mut changed = false;

    for (key, label, fallback) in &settings {
        let current = json
            .get(*key)
            .and_then(|v| v.as_bool())
            .unwrap_or(*fallback);
        print!("  {} [{}]: ", label, if current { "Y/n" } else { "y/N" });
        std::io::stdout().flush()?;
        let mut input = String::new();
        std::io::stdin().read_line(&mut input)?;
        let input_trimmed = input.trim();
        if !input_trimmed.is_empty() {
            let new_val = input_trimmed.eq_ignore_ascii_case("y")
                || input_trimmed.eq_ignore_ascii_case("yes");
            if new_val != current {
                updated_raw = replace_json_bool(&updated_raw, key, new_val);
                changed = true;
            }
        }
    }

    if changed {
        std::fs::write(&fika_config_path, updated_raw)?;
        println!("Fika config updated.");
    } else {
        println!("No changes made.");
    }

    Ok(())
}

/// Scan for unmanaged mods (same as quma init step 4).
fn scan_unmanaged(spt_dir: &Path, db: &Database) -> Result<()> {
    let (unmanaged_dirs, unmanaged_count) = find_unmanaged_mod_dirs(spt_dir, db)?;

    if unmanaged_dirs.is_empty() {
        println!("\nNo unmanaged mod files found.");
    } else {
        println!(
            "\nFound {} unmanaged mod director{} ({} files):",
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
        println!("\nUse `quma track <path> <forge_mod_id>` to associate them with Forge entries.");
    }

    Ok(())
}

fn create_admin_user(spt_dir: &Path, db: &Database, non_interactive: bool) -> Result<()> {
    println!("\n--- Admin User ---");

    if db.admin_exists()? {
        println!("Admin user already exists.");
        return Ok(());
    }

    if non_interactive {
        println!("No admin user created (non-interactive mode).");
        println!("Create one later with the web UI (`quma serve`) or `quma invite`.");
        return Ok(());
    }

    let profiles = list_profiles(spt_dir)?;
    if profiles.is_empty() {
        println!("No SPT profiles found. Start the server and create a profile first.");
        println!("Then run `quma setup` again to create an admin user.");
        return Ok(());
    }

    println!("Select an SPT profile for the admin user:");
    for (i, p) in profiles.iter().enumerate() {
        println!("  [{}] {} (AID: {})", i + 1, p.username, p.aid);
    }

    print!("Select [1-{}]: ", profiles.len());
    std::io::stdout().flush()?;
    let mut input = String::new();
    std::io::stdin().read_line(&mut input)?;
    let choice: usize = input
        .trim()
        .parse()
        .map_err(|_| anyhow::anyhow!("invalid selection"))?;

    if choice == 0 || choice > profiles.len() {
        bail!("selection out of range");
    }

    let profile = &profiles[choice - 1];

    // Prompt for password without echoing to terminal
    let password = rpassword::prompt_password_stdout("Password (min 8 chars): ")
        .context("failed to read password")?;

    if password.len() < 8 {
        bail!("password must be at least 8 characters");
    }

    let password_hash = hash_password(&password)?;

    db.insert_user(
        &profile.username,
        &profile.aid,
        Some(&password_hash),
        Role::Admin,
    )
    .map_err(|e| anyhow::anyhow!("failed to create admin user: {e}"))?;

    println!("Admin user '{}' created.", profile.username);
    Ok(())
}

fn print_summary(config: &Config, spt_dir: &Path, skip_fika: bool) {
    println!("\n=== Setup Complete ===\n");
    println!("SPT directory: {}", spt_dir.display());
    if let Some(ref container) = config.server_container {
        println!("Container: {container}");
    }
    if !skip_fika {
        println!("Fika: installed");
    }
    println!("Web UI: http://{}:{}", config.web_bind, config.web_port);
    println!("\nNext steps:");
    println!("  quma serve              Start the web UI");
    println!("  quma server start       Start the SPT server");
    println!("  quma invite             Generate invite codes for players");
    println!("\nNetwork requirements (for multiplayer):");
    println!("  TCP 6969 inbound        SPT server");
    println!("  UDP 25565 inbound       Fika P2P raids (whoever hosts)");
    println!("  Consider UPnP or VPN as alternatives to port forwarding.");
}

#[cfg(test)]
mod tests {
    use super::*;

    fn create_http_json(spt_dir: &Path, ip: &str, port: u16) -> PathBuf {
        let configs_dir = spt_dir.join("SPT/SPT_Data/configs");
        std::fs::create_dir_all(&configs_dir).unwrap();
        let path = configs_dir.join("http.json");
        std::fs::write(&path, format!(r#"{{"ip": "{ip}", "port": {port}}}"#)).unwrap();
        path
    }

    #[test]
    fn configure_networking_updates_localhost_to_all_interfaces() {
        let tmp = tempfile::tempdir().unwrap();
        create_http_json(tmp.path(), "127.0.0.1", 6969);

        let mut config = Config::default();
        configure_networking(tmp.path(), &mut config, true).unwrap();

        assert_eq!(config.server_host, Some("0.0.0.0".to_string()));
        assert_eq!(config.server_port, Some(6969));

        // Verify the file was actually updated
        let updated =
            std::fs::read_to_string(tmp.path().join("SPT/SPT_Data/configs/http.json")).unwrap();
        let json: serde_json::Value = serde_json::from_str(&updated).unwrap();
        assert_eq!(json["ip"].as_str().unwrap(), "0.0.0.0");
    }

    #[test]
    fn configure_networking_preserves_non_localhost() {
        let tmp = tempfile::tempdir().unwrap();
        create_http_json(tmp.path(), "0.0.0.0", 7000);

        let mut config = Config::default();
        configure_networking(tmp.path(), &mut config, true).unwrap();

        assert_eq!(config.server_host, Some("0.0.0.0".to_string()));
        assert_eq!(config.server_port, Some(7000));
    }

    #[test]
    fn configure_networking_handles_missing_file() {
        let tmp = tempfile::tempdir().unwrap();

        let mut config = Config::default();
        configure_networking(tmp.path(), &mut config, true).unwrap();

        assert_eq!(config.server_host, Some("127.0.0.1".to_string()));
        assert_eq!(config.server_port, Some(6969));
    }

    #[test]
    fn scan_unmanaged_with_empty_db() {
        let tmp = tempfile::tempdir().unwrap();
        let spt_dir = tmp.path();
        // Create mod directories
        std::fs::create_dir_all(spt_dir.join("SPT/user/mods/SomeMod")).unwrap();
        std::fs::write(spt_dir.join("SPT/user/mods/SomeMod/mod.dll"), b"test").unwrap();
        std::fs::create_dir_all(spt_dir.join("BepInEx/plugins")).unwrap();

        let db = Database::open_in_memory().unwrap();
        // Should not panic, should report unmanaged dirs
        scan_unmanaged(spt_dir, &db).unwrap();
    }

    #[test]
    fn create_admin_user_skips_in_non_interactive() {
        let tmp = tempfile::tempdir().unwrap();
        let db = Database::open_in_memory().unwrap();

        // Non-interactive should skip without error
        create_admin_user(tmp.path(), &db, true).unwrap();
        assert!(!db.admin_exists().unwrap());
    }

    #[test]
    fn create_admin_user_skips_when_admin_exists() {
        let tmp = tempfile::tempdir().unwrap();
        let db = Database::open_in_memory().unwrap();
        db.insert_user("admin", "aid123", Some("hash"), Role::Admin)
            .unwrap();

        // Should skip with message, not prompt
        create_admin_user(tmp.path(), &db, false).unwrap();
    }
}
