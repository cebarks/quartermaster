use anyhow::{bail, Context, Result};

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

    let to_install = resolve_deps(ctx, &forge_mod, &selected).await?;
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
    ctx: &CliContext,
) -> Result<()> {
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

    let to_install = resolve_deps(ctx, &forge_mod, &selected_version).await?;
    display_install_plan(&forge_mod.name, &selected_version.version, &to_install);

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

    if force {
        let running = crate::server_detect::is_server_running(
            &ctx.config,
            &ctx.spt_dir,
            ctx.container_mgr.as_ref(),
        )
        .await?;
        if running {
            println!(
                "Warning: applying changes while the server is running may cause instability."
            );
        }
    }

    install_deps(ctx, &to_install).await?;
    let db_id = install_main_mod(ctx, &forge_mod, &selected_version).await?;
    record_dependency_edges(ctx, db_id, &to_install)?;

    println!(
        "\n{} v{} installed successfully.",
        forge_mod.name, selected_version.version
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
        None => versions.into_iter().last().ok_or_else(|| {
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

async fn resolve_deps(
    ctx: &CliContext,
    forge_mod: &crate::forge::models::ForgeMod,
    selected_version: &ForgeVersion,
) -> Result<Vec<PendingInstall>> {
    let dep_nodes = ctx
        .forge
        .get_dependencies(&[(forge_mod.id, &selected_version.version)])
        .await?;

    let mut to_install = Vec::new();
    collect_deps_to_install(&dep_nodes, &ctx.db, &mut to_install)?;
    Ok(to_install)
}

fn display_install_plan(mod_name: &str, mod_version: &str, deps: &[PendingInstall]) {
    println!("\nInstall plan:");
    println!("  {} v{}", mod_name, mod_version);
    for dep in deps {
        println!("  + {} v{} (dependency)", dep.name, dep.version);
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
            dep.mod_id,
            dep.version_id,
            download_url,
            &dep.name,
            dep_mod.slug.as_deref(),
            &dep.version,
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
        forge_mod.id,
        selected_version.id,
        download_url,
        &forge_mod.name,
        forge_mod.slug.as_deref(),
        &selected_version.version,
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

/// Download, extract, and record a single mod in the database.
pub async fn install_single_mod(
    ctx: &CliContext,
    forge_mod_id: i64,
    forge_version_id: i64,
    download_url: &str,
    name: &str,
    slug: Option<&str>,
    version: &str,
) -> Result<i64> {
    if let Some(existing) = ctx.db.get_mod_by_forge_id(forge_mod_id)? {
        println!(
            "  {} already installed (v{}), skipping",
            name, existing.version
        );
        return Ok(existing.id);
    }

    let tmp_dir = tempfile::tempdir().context("failed to create temp directory")?;
    let archive_path = tmp_dir.path().join("mod.zip");
    println!("  Downloading {}...", name);
    ctx.forge.download_file(download_url, &archive_path).await?;

    let mod_type = detect_mod_type(&archive_path)?;
    if mod_type == ModType::Ambiguous {
        println!(
            "  Warning: could not determine mod type for {}. Extracting as-is.",
            name
        );
    }

    println!("  Extracting...");
    let db_id = crate::ops::install_mod_from_archive(
        &ctx.db,
        &ctx.spt_dir,
        &ctx.config,
        forge_mod_id,
        forge_version_id,
        name,
        slug,
        version,
        &archive_path,
    )?;

    let file_count = ctx.db.get_files_for_mod(db_id)?.len();
    println!("  Extracted {} files", file_count);

    Ok(db_id)
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
) -> Result<()> {
    for node in nodes {
        if db.get_mod_by_forge_id(node.id)?.is_some() {
            continue;
        }
        if out.iter().any(|p| p.mod_id == node.id) {
            continue;
        }

        // Recurse into children first so deps install before their parents
        collect_deps_to_install(&node.dependencies, db, out)?;

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
mod tests {
    use super::*;
    use crate::db::Database;
    use crate::forge::models::DependencyNode;

    #[test]
    fn collect_deps_skips_already_installed() {
        use crate::forge::models::ForgeVersion;

        let db = Database::open_in_memory().unwrap();
        db.insert_mod(10, 20, "AlreadyInstalled", None, "1.0.0")
            .unwrap();

        let nodes = vec![DependencyNode {
            id: 10,
            name: "AlreadyInstalled".to_string(),
            slug: None,
            latest_compatible_version: Some(ForgeVersion {
                id: 20,
                version: "1.0.0".to_string(),
                spt_version: None,
                link: None,
                content_length: None,
                fika_compatibility: None,
                dependencies: None,
            }),
            dependencies: vec![],
            conflict: false,
        }];

        let mut out = Vec::new();
        collect_deps_to_install(&nodes, &db, &mut out).unwrap();

        assert!(out.is_empty(), "should skip already-installed deps");
    }

    #[test]
    fn collect_deps_flattens_tree_children_first() {
        use crate::forge::models::ForgeVersion;

        let db = Database::open_in_memory().unwrap();

        let nodes = vec![DependencyNode {
            id: 10,
            name: "Parent".to_string(),
            slug: None,
            latest_compatible_version: Some(ForgeVersion {
                id: 20,
                version: "1.0.0".to_string(),
                spt_version: None,
                link: None,
                content_length: None,
                fika_compatibility: None,
                dependencies: None,
            }),
            dependencies: vec![DependencyNode {
                id: 30,
                name: "Child".to_string(),
                slug: None,
                latest_compatible_version: Some(ForgeVersion {
                    id: 40,
                    version: "0.5.0".to_string(),
                    spt_version: None,
                    link: None,
                    content_length: None,
                    fika_compatibility: None,
                    dependencies: None,
                }),
                dependencies: vec![],
                conflict: false,
            }],
            conflict: false,
        }];

        let mut out = Vec::new();
        collect_deps_to_install(&nodes, &db, &mut out).unwrap();

        assert_eq!(out.len(), 2);
        assert_eq!(out[0].mod_id, 30); // Child first (install order)
        assert_eq!(out[1].mod_id, 10); // Parent second
    }

    #[test]
    fn collect_deps_deduplicates() {
        use crate::forge::models::ForgeVersion;

        let db = Database::open_in_memory().unwrap();

        let shared_dep = DependencyNode {
            id: 99,
            name: "SharedLib".to_string(),
            slug: None,
            latest_compatible_version: Some(ForgeVersion {
                id: 100,
                version: "1.0.0".to_string(),
                spt_version: None,
                link: None,
                content_length: None,
                fika_compatibility: None,
                dependencies: None,
            }),
            dependencies: vec![],
            conflict: false,
        };

        let nodes = vec![
            DependencyNode {
                id: 10,
                name: "ModA".to_string(),
                slug: None,
                latest_compatible_version: Some(ForgeVersion {
                    id: 20,
                    version: "1.0.0".to_string(),
                    spt_version: None,
                    link: None,
                    content_length: None,
                    fika_compatibility: None,
                    dependencies: None,
                }),
                dependencies: vec![shared_dep.clone()],
                conflict: false,
            },
            DependencyNode {
                id: 30,
                name: "ModB".to_string(),
                slug: None,
                latest_compatible_version: Some(ForgeVersion {
                    id: 40,
                    version: "2.0.0".to_string(),
                    spt_version: None,
                    link: None,
                    content_length: None,
                    fika_compatibility: None,
                    dependencies: None,
                }),
                dependencies: vec![DependencyNode {
                    id: 99,
                    name: "SharedLib".to_string(),
                    slug: None,
                    latest_compatible_version: Some(ForgeVersion {
                        id: 100,
                        version: "1.0.0".to_string(),
                        spt_version: None,
                        link: None,
                        content_length: None,
                        fika_compatibility: None,
                        dependencies: None,
                    }),
                    dependencies: vec![],
                    conflict: false,
                }],
                conflict: false,
            },
        ];

        let mut out = Vec::new();
        collect_deps_to_install(&nodes, &db, &mut out).unwrap();

        let shared_count = out.iter().filter(|p| p.mod_id == 99).count();
        assert_eq!(shared_count, 1, "SharedLib should appear only once");
    }
}
