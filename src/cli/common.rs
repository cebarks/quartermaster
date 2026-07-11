use std::io::Write;

use anyhow::{bail, Context, Result};

use crate::config::Config;
use crate::container::ContainerManager;
use crate::db::Database;
use crate::dirs::QumaDirs;
use crate::forge::client::ForgeClient;
use crate::spt::detect::{read_spt_version, SptInfo};

use super::Cli;

pub struct CliContext {
    pub dirs: QumaDirs,
    pub spt_info: SptInfo,
    pub config: Config,
    pub db: Database,
    pub forge: ForgeClient,
    pub container_mgr: Option<ContainerManager>,
}

pub fn resolve_context(cli: &Cli) -> Result<CliContext> {
    let dirs = QumaDirs::detect(cli.effective_quma_dir(), None)?;
    tracing::debug!(quma_dir = %dirs.root.display(), "resolved quma directory");

    // Check for incomplete migration
    let marker = dirs.root.join(".migration-in-progress");
    if marker.exists() {
        bail!(
            "A previous `quma migrate` did not complete. \
             Inspect the directory at {} and re-run `quma migrate`, \
             or remove {} to skip this check.",
            dirs.root.display(),
            marker.display()
        );
    }

    let spt_info = read_spt_version(&dirs.spt_server)?;

    let config_path = Config::resolve_path(cli.config.as_deref(), Some(&dirs.root));
    tracing::debug!(config_path = %config_path.display(), "resolved config path");
    let config = Config::load_with_env(&config_path)
        .with_context(|| format!("failed to load config from {}", config_path.display()))?;

    let db_path = dirs.db_path();
    let db = Database::open(&db_path)
        .with_context(|| format!("failed to open database at {}", db_path.display()))?;

    crate::ops::cleanup_staging(&dirs.root);

    if let Err(e) = crate::ops::migrate_disabled_to_stash(&db, &dirs.root) {
        tracing::error!(err = %e, "failed to migrate disabled mods to stash");
    }

    let forge = ForgeClient::new()?;

    let container_mgr = ContainerManager::new(config.container_stop_timeout).ok();

    Ok(CliContext {
        dirs,
        spt_info,
        config,
        db,
        forge,
        container_mgr,
    })
}

/// Truncate a string to at most `max` characters, appending "…" if truncated.
/// Safe for multi-byte UTF-8.
pub fn truncate_str(s: &str, max: usize) -> String {
    let char_count = s.chars().count();
    if char_count <= max {
        s.to_string()
    } else {
        let end = s.char_indices().nth(max).map(|(i, _)| i).unwrap_or(s.len());
        format!("{}…", &s[..end])
    }
}

/// Resolve a user-provided mod reference (name, slug, or numeric ID) to a ForgeMod.
pub async fn resolve_mod(
    forge: &ForgeClient,
    mod_ref: &str,
) -> Result<crate::forge::models::ForgeMod> {
    if let Ok(id) = mod_ref.parse::<i64>() {
        return forge
            .get_mod(id, false)
            .await
            .with_context(|| format!("mod with ID {id} not found on Forge"));
    }

    let results = forge.search_mods(mod_ref).await?;

    match results.len() {
        0 => bail!("no mods found matching '{mod_ref}' on Forge"),
        1 => Ok(results.into_iter().next().expect("length checked to be 1")),
        _ => {
            if let Some(exact) = results.iter().find(|m| {
                m.name.eq_ignore_ascii_case(mod_ref)
                    || m.slug
                        .as_deref()
                        .is_some_and(|s| s.eq_ignore_ascii_case(mod_ref))
            }) {
                return Ok(exact.clone());
            }

            println!("Multiple mods match '{mod_ref}':");
            for (i, m) in results.iter().enumerate() {
                println!(
                    "  [{}] {} (ID: {}){}",
                    i + 1,
                    m.name,
                    m.id,
                    m.description
                        .as_deref()
                        .map(|d| format!(" — {}", truncate_str(d, 60)))
                        .unwrap_or_default()
                );
            }

            print!("Select [1-{}]: ", results.len());
            std::io::stdout().flush()?;

            let mut input = String::new();
            std::io::stdin().read_line(&mut input)?;
            let choice: usize = input.trim().parse().with_context(|| "invalid selection")?;

            if choice == 0 || choice > results.len() {
                bail!("selection out of range");
            }

            Ok(results
                .into_iter()
                .nth(choice - 1)
                .expect("bounds checked above"))
        }
    }
}

/// Resolve a user-provided mod reference to an installed mod in the database.
pub fn resolve_installed_mod(
    mod_ref: &str,
    ctx: &CliContext,
) -> Result<crate::db::mods::InstalledMod> {
    if let Ok(forge_id) = mod_ref.parse::<i64>() {
        if let Some(m) = ctx.db.get_mod_by_forge_id(forge_id)? {
            return Ok(m);
        }
    }

    if let Some(m) = ctx.db.get_mod_by_name_or_slug(mod_ref)? {
        return Ok(m);
    }

    bail!(
        "mod '{}' is not installed. Run `quma list` to see installed mods.",
        mod_ref
    );
}

/// Resolve a user-provided addon reference (numeric ID, name, or slug) to a Forge addon.
pub async fn resolve_addon(
    forge: &ForgeClient,
    addon_ref: &str,
) -> Result<crate::forge::models::ForgeAddon> {
    if let Ok(id) = addon_ref.parse::<i64>() {
        let addon = forge
            .get_addon(id, false)
            .await
            .with_context(|| format!("addon with ID {id} not found on Forge"))?;

        // Reject detached addons
        if addon.mod_id.is_none() {
            bail!("Detached addons are not supported yet.");
        }

        return Ok(addon);
    }

    let results = forge.search_addons(addon_ref).await?;

    match results.len() {
        0 => bail!("no addons found matching '{addon_ref}' on Forge"),
        1 => {
            let addon = results.into_iter().next().expect("length checked to be 1");
            // Reject detached addons
            if addon.mod_id.is_none() {
                bail!("Detached addons are not supported yet.");
            }
            Ok(addon)
        }
        _ => {
            if let Some(exact) = results.iter().find(|a| {
                a.name.eq_ignore_ascii_case(addon_ref)
                    || a.slug
                        .as_deref()
                        .is_some_and(|s| s.eq_ignore_ascii_case(addon_ref))
            }) {
                // Reject detached addons
                if exact.mod_id.is_none() {
                    bail!("Detached addons are not supported yet.");
                }
                return Ok(exact.clone());
            }

            println!("Multiple addons match '{addon_ref}':");
            for (i, a) in results.iter().enumerate() {
                println!(
                    "  [{}] {} (ID: {}){}",
                    i + 1,
                    a.name,
                    a.id,
                    a.description
                        .as_deref()
                        .map(|d| format!(" — {}", truncate_str(d, 60)))
                        .unwrap_or_default()
                );
            }

            print!("Select [1-{}]: ", results.len());
            std::io::stdout().flush()?;

            let mut input = String::new();
            std::io::stdin().read_line(&mut input)?;
            let choice: usize = input.trim().parse().with_context(|| "invalid selection")?;

            if choice == 0 || choice > results.len() {
                bail!("selection out of range");
            }

            let addon = results
                .into_iter()
                .nth(choice - 1)
                .expect("bounds checked above");

            // Reject detached addons
            if addon.mod_id.is_none() {
                bail!("Detached addons are not supported yet.");
            }

            Ok(addon)
        }
    }
}

/// Resolve a user-provided addon reference to an installed addon in the database.
pub fn resolve_installed_addon(
    addon_ref: &str,
    ctx: &CliContext,
) -> Result<crate::db::addons::InstalledAddon> {
    if let Ok(forge_id) = addon_ref.parse::<i64>() {
        if let Some(a) = ctx.db.get_addon_by_forge_id(forge_id)? {
            return Ok(a);
        }
    }

    if let Some(a) = ctx.db.get_addon_by_name_or_slug(addon_ref)? {
        return Ok(a);
    }

    bail!(
        "addon '{}' is not installed. Run `quma list` to see installed addons.",
        addon_ref
    );
}

/// Group untracked file paths by their mod directory, using appropriate depth for each prefix.
/// - SPT/user/mods/ModName/... -> SPT/user/mods/ModName (4 components)
/// - BepInEx/plugins/ModName/... -> BepInEx/plugins/ModName (3 components)
/// - Other paths -> returned as-is
pub fn group_untracked_by_mod_dir(
    untracked_paths: &[&str],
) -> std::collections::BTreeMap<String, usize> {
    let mut dirs: std::collections::BTreeMap<String, usize> = std::collections::BTreeMap::new();
    for path in untracked_paths {
        let parts: Vec<&str> = path.split('/').collect();
        let dir = if path.starts_with("SPT/") && parts.len() >= 4 {
            format!("{}/{}/{}/{}", parts[0], parts[1], parts[2], parts[3])
        } else if path.starts_with("BepInEx/") && parts.len() >= 3 {
            format!("{}/{}/{}", parts[0], parts[1], parts[2])
        } else {
            path.to_string()
        };
        *dirs.entry(dir).or_default() += 1;
    }
    dirs
}

/// Scan for unmanaged mod files (on disk but not in DB) and group by top-level mod directory.
pub fn find_unmanaged_mod_dirs(
    dirs: &QumaDirs,
    db: &Database,
) -> Result<(std::collections::BTreeMap<String, usize>, usize)> {
    use crate::spt::mods::scan_mod_directories;

    let all_files_on_disk = scan_mod_directories(&dirs.spt_server)?;
    let tracked_files = db.get_all_tracked_files()?;
    let tracked_paths: std::collections::HashSet<&str> =
        tracked_files.iter().map(|f| f.file_path.as_str()).collect();

    let unmanaged: Vec<&str> = all_files_on_disk
        .iter()
        .filter(|f| !tracked_paths.contains(f.as_str()))
        .map(|f| f.as_str())
        .collect();

    let total = unmanaged.len();
    let mut dirs_map = group_untracked_by_mod_dir(&unmanaged);

    // Build set of mod directories that have any tracked files
    let managed_dirs = group_untracked_by_mod_dir(
        &tracked_files
            .iter()
            .map(|f| f.file_path.as_str())
            .collect::<Vec<_>>(),
    );

    // Exclude directories that belong to tracked mods (they have runtime-generated
    // files but the mod itself is managed by quartermaster)
    dirs_map.retain(|dir, _| !managed_dirs.contains_key(dir));

    // Exclude core SPT directories — these ship with the server, not from mods
    dirs_map.remove("BepInEx/plugins/spt");

    Ok((dirs_map, total))
}

/// Warn the user if `--force` is being used while the server is running.
pub async fn warn_if_forcing_while_running(force: bool, ctx: &CliContext) -> Result<()> {
    if force {
        let running = crate::server_detect::is_server_running(
            &ctx.config,
            &ctx.dirs.root,
            ctx.container_mgr.as_ref(),
        )
        .await?;
        if running {
            println!(
                "Warning: applying changes while the server is running may cause instability."
            );
        }
    }
    Ok(())
}

/// Prompt the user for yes/no confirmation. Returns true if confirmed.
pub fn confirm(prompt: &str) -> Result<bool> {
    print!("{} [y/N]: ", prompt);
    std::io::stdout().flush()?;
    let mut input = String::new();
    std::io::stdin().read_line(&mut input)?;
    Ok(input.trim().eq_ignore_ascii_case("y"))
}

/// Check if a version satisfies a constraint string.
/// Without a semver library, this does basic exact matching for plain version strings
/// and assumes compatibility for operator-based constraints (logging at debug level).
pub fn version_satisfies_constraint(version: &str, constraint: &str) -> bool {
    let trimmed = constraint.trim();
    // If constraint is an exact version string (no operators), do exact match
    if trimmed.chars().next().is_none_or(|c| c.is_ascii_digit()) {
        return trimmed == version;
    }
    // For constraints with operators (~, ^, >=, etc.), we can't evaluate them
    // without a semver library. Log at debug level and assume compatible.
    tracing::debug!(
        version,
        constraint,
        "cannot evaluate semver constraint without semver library, assuming compatible"
    );
    true
}
