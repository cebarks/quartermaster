// Functions in this module are used by subsequent CLI command implementations (tasks 8-12).
// Allow dead_code for now as we're incrementally implementing the CLI.
#![allow(dead_code)]

use std::io::Write;
use std::path::{Path, PathBuf};

use anyhow::{bail, Context, Result};

use crate::config::Config;
use crate::db::Database;
use crate::forge::client::ForgeClient;
use crate::spt::detect::{detect_spt_dir, read_spt_version, SptInfo};

use super::Cli;

pub struct CliContext {
    pub spt_dir: PathBuf,
    pub spt_info: SptInfo,
    pub config: Config,
    pub config_path: PathBuf,
    pub db: Database,
    pub forge: ForgeClient,
}

pub fn resolve_context(cli: &Cli) -> Result<CliContext> {
    let spt_dir = detect_spt_dir(cli.spt_dir.as_deref(), None)?;
    let spt_info = read_spt_version(&spt_dir)?;

    let config_path = Config::resolve_path(cli.config.as_deref(), Some(&spt_dir));
    let config = Config::load_with_env(&config_path)
        .with_context(|| format!("failed to load config from {}", config_path.display()))?;

    let db_path = spt_dir.join("quartermaster.db");
    let db = Database::open(&db_path)
        .with_context(|| format!("failed to open database at {}", db_path.display()))?;

    let forge = ForgeClient::new(config.forge_token.clone())?;

    Ok(CliContext {
        spt_dir,
        spt_info,
        config,
        config_path,
        db,
        forge,
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
        1 => Ok(results.into_iter().next().unwrap()),
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

            Ok(results.into_iter().nth(choice - 1).unwrap())
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

/// Scan for unmanaged mod files (on disk but not in DB) and group by top-level mod directory.
pub fn find_unmanaged_mod_dirs(
    spt_dir: &Path,
    db: &Database,
) -> Result<(std::collections::BTreeMap<String, usize>, usize)> {
    use crate::spt::mods::scan_mod_directories;

    let all_files_on_disk = scan_mod_directories(spt_dir)?;
    let tracked_files = db.get_all_tracked_files()?;
    let tracked_paths: std::collections::HashSet<&str> =
        tracked_files.iter().map(|f| f.file_path.as_str()).collect();

    let unmanaged: Vec<&str> = all_files_on_disk
        .iter()
        .filter(|f| !tracked_paths.contains(f.as_str()))
        .map(|f| f.as_str())
        .collect();

    let total = unmanaged.len();
    let mut dirs: std::collections::BTreeMap<String, usize> = std::collections::BTreeMap::new();
    for path in &unmanaged {
        let parts: Vec<&str> = path.split('/').collect();
        if parts.len() >= 3 {
            let dir = format!("{}/{}/{}", parts[0], parts[1], parts[2]);
            *dirs.entry(dir).or_default() += 1;
        }
    }

    Ok((dirs, total))
}

/// Prompt the user for yes/no confirmation. Returns true if confirmed.
pub fn confirm(prompt: &str) -> Result<bool> {
    print!("{} [y/N]: ", prompt);
    std::io::stdout().flush()?;
    let mut input = String::new();
    std::io::stdin().read_line(&mut input)?;
    Ok(input.trim().eq_ignore_ascii_case("y"))
}
