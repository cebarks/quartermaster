use std::path::Path;

use anyhow::{bail, Context, Result};

use crate::config::Config;
use crate::db::Database;
use crate::forge::client::ForgeClient;
use crate::forge::models::{DependencyNode, FikaCompat, ForgeVersion};
use crate::spt::mods::{detect_mod_type, ModType};

use super::common::{confirm, resolve_mod, CliContext};

/// Resolve dependencies and install a mod plus all its deps.
/// Used by both the interactive `install` command and `apply::drain_all`.
pub async fn install_with_deps(ctx: &CliContext, forge_mod_id: i64, version_id: i64) -> Result<()> {
    let forge_mod = ctx.forge.get_mod(forge_mod_id, false).await?;

    if let Some(existing) = ctx.db.get_mod_by_forge_id(forge_mod.id)? {
        println!(
            "  {} already installed (v{}), skipping",
            existing.name, existing.version
        );
        return Ok(());
    }

    let versions = ctx.forge.get_versions(forge_mod_id, None).await?;

    let selected = versions
        .iter()
        .find(|v| v.id == version_id)
        .ok_or_else(|| {
            anyhow::anyhow!(
                "version ID {} not found for {} on Forge",
                version_id,
                forge_mod.name
            )
        })?
        .clone();

    let (to_install, skipped_conflicts) = resolve_deps(ctx, &forge_mod, &selected).await?;
    if !skipped_conflicts.is_empty() {
        tracing::warn!(
            "Skipped {} conflicting dependency(ies) for {}: {}",
            skipped_conflicts.len(),
            forge_mod.name,
            skipped_conflicts.join(", "),
        );
    }
    install_deps(ctx, &to_install).await?;
    let db_id = install_main_mod(ctx, &forge_mod, &selected).await?;
    record_dependency_edges(ctx, db_id, &to_install)?;

    println!(
        "  {} v{} installed successfully.",
        forge_mod.name, selected.version
    );
    Ok(())
}

/// A dependency that needs to be installed.
struct PendingInstall {
    mod_id: i64,
    version_id: i64,
    name: String,
    version: String,
}

pub async fn run(
    mod_ref: &str,
    version: Option<&str>,
    force: bool,
    addon: bool,
    ctx: &CliContext,
) -> Result<()> {
    // If --addon flag is set, use addon install flow
    if addon {
        return run_addon_install(mod_ref, version, force, ctx).await;
    }

    let forge_mod = resolve_mod(&ctx.forge, mod_ref).await?;
    println!("Found: {} (ID: {})", forge_mod.name, forge_mod.id);

    if let Some(existing) = ctx.db.get_mod_by_forge_id(forge_mod.id)? {
        bail!(
            "{} is already installed (version {}). Use `quma update` to update it.",
            existing.name,
            existing.version
        );
    }

    let selected_version = pick_version(ctx, &forge_mod, version).await?;
    check_fika_compat(&forge_mod.name, &selected_version)?;

    let (to_install, skipped_conflicts) = resolve_deps(ctx, &forge_mod, &selected_version).await?;
    display_install_plan(
        &forge_mod.name,
        &selected_version.version,
        &to_install,
        &skipped_conflicts,
    );

    if !confirm("Proceed with installation?")? {
        println!("Installation cancelled.");
        return Ok(());
    }

    if crate::queue::should_queue(&ctx.config, force, &ctx.spt_dir, ctx.container_mgr.as_ref())
        .await?
    {
        ctx.db.insert_pending_op(
            crate::db::users::QueueAction::Install,
            forge_mod.id,
            Some(selected_version.id),
            &forge_mod.name,
            None,
            None,
        )?;
        println!(
            "Server is running — operation queued. It will be applied on next server restart."
        );
        return Ok(());
    }

    super::common::warn_if_forcing_while_running(force, ctx).await?;

    install_deps(ctx, &to_install).await?;
    let db_id = install_main_mod(ctx, &forge_mod, &selected_version).await?;
    record_dependency_edges(ctx, db_id, &to_install)?;

    println!(
        "\n{} v{} installed successfully.",
        forge_mod.name, selected_version.version
    );
    Ok(())
}

async fn run_addon_install(
    addon_ref: &str,
    version: Option<&str>,
    force: bool,
    ctx: &CliContext,
) -> Result<()> {
    use super::common::resolve_addon;

    let forge_addon = resolve_addon(&ctx.forge, addon_ref).await?;
    println!("Found: {} (ID: {})", forge_addon.name, forge_addon.id);

    // Check if addon is already installed
    if let Some(existing) = ctx.db.get_addon_by_forge_id(forge_addon.id)? {
        bail!(
            "{} is already installed (version {}). Use `quma update --addon` to update it.",
            existing.name,
            existing.version
        );
    }

    // Resolve parent mod
    let parent_forge_mod_id = forge_addon
        .mod_id
        .ok_or_else(|| anyhow::anyhow!("detached addons are not supported"))?;
    let parent_mod = ctx.db.get_mod_by_forge_id(parent_forge_mod_id)?;

    if parent_mod.is_none() {
        anyhow::bail!(
            "Parent mod (Forge ID: {}) is not installed. Install it first before adding addons.",
            parent_forge_mod_id
        );
    }

    let selected_version = pick_addon_version(ctx, &forge_addon, version).await?;

    // Check mod_version_constraint against parent version if parent is installed
    if let Some(ref parent) = parent_mod {
        if let Some(constraint) = &selected_version.mod_version_constraint {
            if !super::common::version_satisfies_constraint(&parent.version, constraint) {
                tracing::warn!(
                    "Addon version constraint '{}' does not match parent mod version '{}'",
                    constraint,
                    parent.version
                );
            }
        }
    }

    if !confirm("Proceed with installation?")? {
        println!("Installation cancelled.");
        return Ok(());
    }

    if crate::queue::should_queue(&ctx.config, force, &ctx.spt_dir, ctx.container_mgr.as_ref())
        .await?
    {
        ctx.db.insert_pending_addon_op(
            crate::db::users::QueueAction::Install,
            forge_addon.id,
            Some(selected_version.id),
            &forge_addon.name,
            None,
            None,
        )?;
        println!(
            "Server is running — operation queued. It will be applied on next server restart."
        );
        return Ok(());
    }

    super::common::warn_if_forcing_while_running(force, ctx).await?;

    // Download and install
    let download_url = selected_version
        .link
        .as_deref()
        .ok_or_else(|| anyhow::anyhow!("no download URL for addon version"))?;

    println!(
        "Downloading {} v{}...",
        forge_addon.name, selected_version.version
    );
    let tmp_dir = tempfile::tempdir()?;
    let archive_path = tmp_dir.path().join("addon.zip");
    ctx.forge.download_file(download_url, &archive_path).await?;

    let parent_mod_id = parent_mod
        .as_ref()
        .map(|m| m.id)
        .ok_or_else(|| anyhow::anyhow!("parent mod must be installed to install addon"))?;

    crate::ops::install_addon_from_archive(&crate::ops::InstallAddonRequest {
        db: &ctx.db,
        spt_dir: &ctx.spt_dir,
        config: &ctx.config,
        forge_addon_id: forge_addon.id,
        parent_mod_id,
        version_id: selected_version.id,
        name: &forge_addon.name,
        slug: forge_addon.slug.as_deref(),
        version: &selected_version.version,
        mod_version_constraint: selected_version.mod_version_constraint.as_deref(),
        archive_path: &archive_path,
    })?;

    println!(
        "\n{} v{} installed successfully.",
        forge_addon.name, selected_version.version
    );
    Ok(())
}

async fn pick_version(
    ctx: &CliContext,
    forge_mod: &crate::forge::models::ForgeMod,
    explicit_version: Option<&str>,
) -> Result<ForgeVersion> {
    let versions = ctx
        .forge
        .get_versions(forge_mod.id, Some(&ctx.spt_info.spt_version))
        .await?;

    let selected = match explicit_version {
        Some(ver) => {
            // If explicit version doesn't match any SPT-compatible version,
            // try fetching all versions unfiltered
            let found = versions.iter().find(|v| v.version == ver);
            match found {
                Some(v) => v.clone(),
                None => {
                    let all_versions = ctx.forge.get_versions(forge_mod.id, None).await?;
                    let v = all_versions
                        .into_iter()
                        .find(|v| v.version == ver)
                        .ok_or_else(|| {
                            anyhow::anyhow!(
                                "version '{}' not found for {} on Forge",
                                ver,
                                forge_mod.name
                            )
                        })?;
                    println!(
                        "Warning: {} v{} is not listed as compatible with SPT {}.",
                        forge_mod.name, ver, ctx.spt_info.spt_version
                    );
                    v
                }
            }
        }
        None => versions.into_iter().max_by_key(|v| v.id).ok_or_else(|| {
            anyhow::anyhow!(
                "no versions of {} are compatible with SPT {}",
                forge_mod.name,
                ctx.spt_info.spt_version
            )
        })?,
    };

    println!(
        "Selected version: {} (SPT {})",
        selected.version,
        selected.spt_version.as_deref().unwrap_or("unknown")
    );
    Ok(selected)
}

async fn pick_addon_version(
    ctx: &CliContext,
    forge_addon: &crate::forge::models::ForgeAddon,
    explicit_version: Option<&str>,
) -> Result<crate::forge::models::ForgeAddonVersion> {
    let versions = ctx.forge.get_addon_versions(forge_addon.id).await?;

    if versions.is_empty() {
        bail!("no versions available for addon {}", forge_addon.name);
    }

    let selected = match explicit_version {
        Some(ver) => {
            let found = versions
                .into_iter()
                .find(|v| v.version == ver)
                .ok_or_else(|| {
                    anyhow::anyhow!(
                        "version '{}' not found for {} on Forge",
                        ver,
                        forge_addon.name
                    )
                })?;
            found
        }
        None => {
            // Return the latest version (first in list)
            versions
                .into_iter()
                .next()
                .expect("checked non-empty above")
        }
    };

    println!("Selected version: {}", selected.version);
    Ok(selected)
}

async fn resolve_deps(
    ctx: &CliContext,
    forge_mod: &crate::forge::models::ForgeMod,
    selected_version: &ForgeVersion,
) -> Result<(Vec<PendingInstall>, Vec<String>)> {
    let dep_nodes = ctx
        .forge
        .get_dependencies(&[(&forge_mod.id.to_string(), &selected_version.version)])
        .await?;

    let mut to_install = Vec::new();
    let mut skipped_conflicts = Vec::new();
    collect_deps_to_install(&dep_nodes, &ctx.db, &mut to_install, &mut skipped_conflicts)?;
    Ok((to_install, skipped_conflicts))
}

fn display_install_plan(
    mod_name: &str,
    mod_version: &str,
    deps: &[PendingInstall],
    skipped_conflicts: &[String],
) {
    println!("\nInstall plan:");
    println!("  {} v{}", mod_name, mod_version);
    for dep in deps {
        println!("  + {} v{} (dependency)", dep.name, dep.version);
    }
    if !skipped_conflicts.is_empty() {
        println!("\n  Skipped (conflicts):");
        for name in skipped_conflicts {
            println!("    - {} (marked as conflict by Forge)", name);
        }
    }
}

async fn install_deps(ctx: &CliContext, deps: &[PendingInstall]) -> Result<()> {
    for dep in deps {
        println!("\nInstalling dependency: {} v{}", dep.name, dep.version);
        let dep_versions = ctx.forge.get_versions(dep.mod_id, None).await?;

        let dep_version = dep_versions
            .iter()
            .find(|v| v.id == dep.version_id)
            .ok_or_else(|| {
                anyhow::anyhow!(
                    "version {} for dependency {} not found on Forge (may have been delisted)",
                    dep.version_id,
                    dep.name
                )
            })?;

        let download_url = dep_version
            .link
            .as_deref()
            .ok_or_else(|| anyhow::anyhow!("no download link for {} v{}", dep.name, dep.version))?;
        let dep_mod = ctx.forge.get_mod(dep.mod_id, false).await?;
        install_single_mod(
            ctx,
            &ModInstallParams {
                forge_mod_id: dep.mod_id,
                forge_version_id: dep.version_id,
                download_url,
                name: &dep.name,
                slug: dep_mod.slug.as_deref(),
                version: &dep.version,
            },
        )
        .await?;
    }
    Ok(())
}

async fn install_main_mod(
    ctx: &CliContext,
    forge_mod: &crate::forge::models::ForgeMod,
    selected_version: &ForgeVersion,
) -> Result<i64> {
    let download_url = selected_version.link.as_deref().ok_or_else(|| {
        anyhow::anyhow!(
            "no download link for {} v{}",
            forge_mod.name,
            selected_version.version
        )
    })?;

    install_single_mod(
        ctx,
        &ModInstallParams {
            forge_mod_id: forge_mod.id,
            forge_version_id: selected_version.id,
            download_url,
            name: &forge_mod.name,
            slug: forge_mod.slug.as_deref(),
            version: &selected_version.version,
        },
    )
    .await
}

fn record_dependency_edges(
    ctx: &CliContext,
    main_mod_db_id: i64,
    deps: &[PendingInstall],
) -> Result<()> {
    for dep in deps {
        let dep_installed = ctx.db.get_mod_by_forge_id(dep.mod_id)?;
        let dep_db_id = match dep_installed {
            Some(m) => m.id,
            None => continue,
        };

        match ctx.db.insert_dependency(main_mod_db_id, dep_db_id, None) {
            Ok(_) => {}
            Err(rusqlite::Error::SqliteFailure(err, _))
                if err.code == rusqlite::ffi::ErrorCode::ConstraintViolation => {}
            Err(e) => return Err(e.into()),
        }
    }

    Ok(())
}

/// Metadata needed to install a single mod from Forge.
pub struct ModInstallParams<'a> {
    pub forge_mod_id: i64,
    pub forge_version_id: i64,
    pub download_url: &'a str,
    pub name: &'a str,
    pub slug: Option<&'a str>,
    pub version: &'a str,
}

/// Download a mod archive from Forge, extract it, and record it in the database.
///
/// This is the shared core of mod installation used by both the CLI `install`
/// command and the setup wizard's NarcoNet installer. It handles:
/// 1. Downloading the archive to a temp directory
/// 2. Detecting mod type and warning on ambiguous archives
/// 3. Extracting via `ops::install_mod_from_archive`
/// 4. Reporting the installed file count
pub async fn download_and_install(
    forge: &ForgeClient,
    db: &Database,
    spt_dir: &Path,
    config: &Config,
    params: &ModInstallParams<'_>,
) -> Result<i64> {
    let ModInstallParams {
        forge_mod_id,
        forge_version_id,
        download_url,
        name,
        slug,
        version,
    } = params;

    let tmp_dir = tempfile::tempdir().context("failed to create temp directory")?;
    let archive_path = tmp_dir.path().join("mod.zip");
    tracing::info!(name, "downloading mod");
    forge.download_file(download_url, &archive_path).await?;

    let mod_type = detect_mod_type(&archive_path)?;
    if mod_type == ModType::Ambiguous {
        tracing::warn!(name, "could not determine mod type, extracting as-is");
    }

    tracing::info!(name, "extracting mod");
    let db_id = crate::ops::install_mod_from_archive(&crate::ops::InstallRequest {
        db,
        spt_dir,
        config,
        forge_mod_id: *forge_mod_id,
        version_id: *forge_version_id,
        name,
        slug: *slug,
        version,
        archive_path: &archive_path,
    })?;

    let file_count = db.get_files_for_mod(db_id)?.len();
    tracing::info!(name, file_count, "mod extracted");

    Ok(db_id)
}

/// Variant of `download_and_install` that accepts an Arc<Mutex<Database>>.
///
/// Used by async contexts (web handlers, ops) where the database is wrapped
/// in Arc<Mutex> for shared access. The mutex is only locked for the
/// synchronous DB operations, not held across the async download.
pub async fn download_and_install_with_arc(
    forge: &ForgeClient,
    db: &std::sync::Arc<parking_lot::Mutex<Database>>,
    spt_dir: &Path,
    config: &Config,
    params: &ModInstallParams<'_>,
) -> Result<i64> {
    let ModInstallParams {
        forge_mod_id,
        forge_version_id,
        download_url,
        name,
        slug,
        version,
    } = params;

    let tmp_dir = tempfile::tempdir().context("failed to create temp directory")?;
    let archive_path = tmp_dir.path().join("mod.zip");
    tracing::info!(name, "downloading mod");
    forge.download_file(download_url, &archive_path).await?;

    let mod_type = detect_mod_type(&archive_path)?;
    if mod_type == ModType::Ambiguous {
        tracing::warn!(name, "could not determine mod type, extracting as-is");
    }

    tracing::info!(name, "extracting mod");
    let db_id = {
        let db_guard = db.lock();
        crate::ops::install_mod_from_archive(&crate::ops::InstallRequest {
            db: &db_guard,
            spt_dir,
            config,
            forge_mod_id: *forge_mod_id,
            version_id: *forge_version_id,
            name,
            slug: *slug,
            version,
            archive_path: &archive_path,
        })?
    };

    let file_count = db.lock().get_files_for_mod(db_id)?.len();
    tracing::info!(name, file_count, "mod extracted");

    Ok(db_id)
}

/// Download, extract, and record a single mod in the database.
/// Skips installation if the mod is already present.
pub async fn install_single_mod(ctx: &CliContext, params: &ModInstallParams<'_>) -> Result<i64> {
    if let Some(existing) = ctx.db.get_mod_by_forge_id(params.forge_mod_id)? {
        println!(
            "  {} already installed (v{}), skipping",
            params.name, existing.version
        );
        return Ok(existing.id);
    }

    download_and_install(&ctx.forge, &ctx.db, &ctx.spt_dir, &ctx.config, params).await
}

fn check_fika_compat(mod_name: &str, version: &ForgeVersion) -> Result<()> {
    match &version.fika_compatibility {
        Some(FikaCompat::Incompatible) => {
            println!(
                "Warning: {} v{} is marked as Fika INCOMPATIBLE.",
                mod_name, version.version,
            );
            if !confirm("Continue anyway?")? {
                bail!("installation cancelled due to Fika incompatibility");
            }
        }
        Some(FikaCompat::Unknown) => {
            println!(
                "Note: Fika compatibility for {} v{} is unknown.",
                mod_name, version.version
            );
        }
        _ => {}
    }
    Ok(())
}

fn collect_deps_to_install(
    nodes: &[DependencyNode],
    db: &crate::db::Database,
    out: &mut Vec<PendingInstall>,
    skipped_conflicts: &mut Vec<String>,
) -> Result<()> {
    for node in nodes {
        // Check conflicts BEFORE the already-installed check so we always
        // surface warnings about conflicting mods, even if they're installed.
        if node.conflict {
            if db.get_mod_by_forge_id(node.id)?.is_some() {
                tracing::warn!(
                    "Dependency '{}' conflicts with this mod and is already installed — you may experience issues",
                    node.name,
                );
            } else {
                tracing::warn!(
                    "Skipping dependency '{}' — marked as a conflict by Forge",
                    node.name,
                );
            }
            skipped_conflicts.push(node.name.clone());
            continue;
        }

        if db.get_mod_by_forge_id(node.id)?.is_some() {
            continue;
        }
        if out.iter().any(|p| p.mod_id == node.id) {
            continue;
        }

        // Recurse into children first so deps install before their parents
        collect_deps_to_install(&node.dependencies, db, out, skipped_conflicts)?;

        // Extract version_id and version string from latest_compatible_version
        let (version_id, version) = match &node.latest_compatible_version {
            Some(v) => (v.id, v.version.clone()),
            None => {
                anyhow::bail!(
                    "dependency {} has no compatible version available",
                    node.name
                );
            }
        };

        out.push(PendingInstall {
            mod_id: node.id,
            version_id,
            name: node.name.clone(),
            version,
        });
    }
    Ok(())
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;
    use crate::db::Database;
    use crate::forge::models::DependencyNode;

    /// Helper to build a simple `DependencyNode` with minimal boilerplate.
    fn dep_node(id: i64, name: &str, version_id: i64, ver: &str, conflict: bool) -> DependencyNode {
        use crate::forge::models::ForgeVersion;
        DependencyNode {
            id,
            guid: None,
            name: name.to_string(),
            slug: None,
            latest_compatible_version: Some(ForgeVersion {
                id: version_id,
                hub_id: None,
                version: ver.to_string(),
                description: None,
                spt_version: None,
                link: None,
                content_length: None,
                downloads: None,
                fika_compatibility: None,
                dependencies: None,
                published_at: None,
                created_at: None,
                updated_at: None,
            }),
            dependencies: vec![],
            conflict,
        }
    }

    #[test]
    fn collect_deps_skips_already_installed() {
        let db = Database::open_in_memory().unwrap();
        db.insert_mod(10, 20, "AlreadyInstalled", None, "1.0.0")
            .unwrap();

        let nodes = vec![dep_node(10, "AlreadyInstalled", 20, "1.0.0", false)];

        let mut out = Vec::new();
        let mut skipped = Vec::new();
        collect_deps_to_install(&nodes, &db, &mut out, &mut skipped).unwrap();

        assert!(out.is_empty(), "should skip already-installed deps");
        assert!(skipped.is_empty());
    }

    #[test]
    fn collect_deps_flattens_tree_children_first() {
        let db = Database::open_in_memory().unwrap();

        let mut parent = dep_node(10, "Parent", 20, "1.0.0", false);
        parent.dependencies = vec![dep_node(30, "Child", 40, "0.5.0", false)];

        let nodes = vec![parent];

        let mut out = Vec::new();
        let mut skipped = Vec::new();
        collect_deps_to_install(&nodes, &db, &mut out, &mut skipped).unwrap();

        assert_eq!(out.len(), 2);
        assert_eq!(out[0].mod_id, 30); // Child first (install order)
        assert_eq!(out[1].mod_id, 10); // Parent second
        assert!(skipped.is_empty());
    }

    #[test]
    fn collect_deps_deduplicates() {
        let db = Database::open_in_memory().unwrap();

        let shared_dep = dep_node(99, "SharedLib", 100, "1.0.0", false);

        let mut mod_a = dep_node(10, "ModA", 20, "1.0.0", false);
        mod_a.dependencies = vec![shared_dep.clone()];

        let mut mod_b = dep_node(30, "ModB", 40, "2.0.0", false);
        mod_b.dependencies = vec![dep_node(99, "SharedLib", 100, "1.0.0", false)];

        let nodes = vec![mod_a, mod_b];

        let mut out = Vec::new();
        let mut skipped = Vec::new();
        collect_deps_to_install(&nodes, &db, &mut out, &mut skipped).unwrap();

        let shared_count = out.iter().filter(|p| p.mod_id == 99).count();
        assert_eq!(shared_count, 1, "SharedLib should appear only once");
        assert!(skipped.is_empty());
    }

    #[test]
    fn collect_deps_skips_conflicts() {
        let db = Database::open_in_memory().unwrap();

        let nodes = vec![
            dep_node(10, "GoodDep", 20, "1.0.0", false),
            dep_node(30, "ConflictingMod", 40, "2.0.0", true),
        ];

        let mut out = Vec::new();
        let mut skipped = Vec::new();
        collect_deps_to_install(&nodes, &db, &mut out, &mut skipped).unwrap();

        assert_eq!(out.len(), 1);
        assert_eq!(out[0].mod_id, 10);
        assert_eq!(skipped, vec!["ConflictingMod"]);
    }

    #[test]
    fn collect_deps_skips_conflict_subtree() {
        let db = Database::open_in_memory().unwrap();

        let mut conflict_parent = dep_node(10, "ConflictParent", 20, "1.0.0", true);
        conflict_parent.dependencies = vec![dep_node(30, "ChildOfConflict", 40, "0.5.0", false)];

        let nodes = vec![conflict_parent];

        let mut out = Vec::new();
        let mut skipped = Vec::new();
        collect_deps_to_install(&nodes, &db, &mut out, &mut skipped).unwrap();

        assert!(
            out.is_empty(),
            "children of conflict should also be skipped"
        );
        assert_eq!(skipped, vec!["ConflictParent"]);
    }

    #[test]
    fn collect_deps_mixed_conflict_siblings() {
        let db = Database::open_in_memory().unwrap();

        // Tree:
        //   Parent (ok)
        //     ├── ChildA (conflict)
        //     │     └── Grandchild (ok, but unreachable)
        //     └── ChildB (ok)
        let mut child_a = dep_node(20, "ChildA", 21, "1.0.0", true);
        child_a.dependencies = vec![dep_node(40, "Grandchild", 41, "1.0.0", false)];

        let child_b = dep_node(30, "ChildB", 31, "1.0.0", false);

        let mut parent = dep_node(10, "Parent", 11, "1.0.0", false);
        parent.dependencies = vec![child_a, child_b];

        let nodes = vec![parent];

        let mut out = Vec::new();
        let mut skipped = Vec::new();
        collect_deps_to_install(&nodes, &db, &mut out, &mut skipped).unwrap();

        let installed_ids: Vec<i64> = out.iter().map(|p| p.mod_id).collect();
        assert_eq!(installed_ids, vec![30, 10], "ChildB then Parent");
        assert!(
            !installed_ids.contains(&20),
            "ChildA (conflict) should not be installed"
        );
        assert!(
            !installed_ids.contains(&40),
            "Grandchild under conflict should not be installed"
        );
        assert_eq!(skipped, vec!["ChildA"]);
    }

    #[test]
    fn collect_deps_conflict_already_installed() {
        let db = Database::open_in_memory().unwrap();
        db.insert_mod(10, 20, "InstalledConflict", None, "1.0.0")
            .unwrap();

        let nodes = vec![dep_node(10, "InstalledConflict", 20, "1.0.0", true)];

        let mut out = Vec::new();
        let mut skipped = Vec::new();
        collect_deps_to_install(&nodes, &db, &mut out, &mut skipped).unwrap();

        // Should still be skipped from install (already installed), but surfaced as a conflict
        assert!(out.is_empty());
        assert_eq!(skipped, vec!["InstalledConflict"]);
    }

    #[test]
    fn collect_deps_shared_dep_via_conflict_and_nonconflict() {
        // SharedLib appears under both a conflict subtree (skipped) and a
        // non-conflict subtree (reachable). It should still be installed.
        let db = Database::open_in_memory().unwrap();

        let mut conflict_node = dep_node(10, "ConflictMod", 11, "1.0.0", true);
        conflict_node.dependencies = vec![dep_node(99, "SharedLib", 100, "1.0.0", false)];

        let mut good_node = dep_node(20, "GoodMod", 21, "1.0.0", false);
        good_node.dependencies = vec![dep_node(99, "SharedLib", 100, "1.0.0", false)];

        let nodes = vec![conflict_node, good_node];

        let mut out = Vec::new();
        let mut skipped = Vec::new();
        collect_deps_to_install(&nodes, &db, &mut out, &mut skipped).unwrap();

        let installed_ids: Vec<i64> = out.iter().map(|p| p.mod_id).collect();
        assert!(
            installed_ids.contains(&99),
            "SharedLib should be installed via non-conflict path"
        );
        assert!(installed_ids.contains(&20), "GoodMod should be installed");
        assert!(
            !installed_ids.contains(&10),
            "ConflictMod should not be installed"
        );
        assert_eq!(skipped, vec!["ConflictMod"]);
    }
}
