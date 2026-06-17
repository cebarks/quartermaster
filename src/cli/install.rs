use anyhow::{bail, Context, Result};

use crate::forge::models::{DependencyNode, FikaCompat, ForgeVersion};
use crate::spt::mods::{detect_mod_type, extract_mod, ModType};

use super::common::{confirm, resolve_mod, CliContext};

/// A dependency that needs to be installed.
struct PendingInstall {
    mod_id: i64,
    version_id: i64,
    name: String,
    version: String,
}

pub async fn run(mod_ref: &str, _force: bool, ctx: &CliContext) -> Result<()> {
    // TODO(debt): _force is accepted but unused until Phase 3 wires server-running detection
    let forge_mod = resolve_mod(&ctx.forge, mod_ref).await?;
    println!("Found: {} (ID: {})", forge_mod.name, forge_mod.id);

    if let Some(existing) = ctx.db.get_mod_by_forge_id(forge_mod.id)? {
        bail!(
            "{} is already installed (version {}). Use `quma update` to update it.",
            existing.name,
            existing.version
        );
    }

    let selected_version = pick_version(ctx, &forge_mod).await?;
    check_fika_compat(&forge_mod.name, &selected_version)?;

    let to_install = resolve_deps(ctx, &forge_mod, &selected_version).await?;
    display_install_plan(&forge_mod.name, &selected_version.version, &to_install);

    if !confirm("Proceed with installation?")? {
        println!("Installation cancelled.");
        return Ok(());
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
) -> Result<ForgeVersion> {
    let versions = ctx
        .forge
        .get_versions(forge_mod.id, Some(&ctx.spt_info.spt_version))
        .await?;

    // TODO: accept explicit version arg when we refactor CLI dispatch
    let selected = versions.into_iter().next().ok_or_else(|| {
        anyhow::anyhow!(
            "no versions of {} are compatible with SPT {}",
            forge_mod.name,
            ctx.spt_info.spt_version
        )
    })?;

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
        .get_dependencies(&[(forge_mod.id, selected_version.id)])
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
    let extracted_files = extract_mod(&archive_path, &ctx.spt_dir)?;
    println!("  Extracted {} files", extracted_files.len());

    let db_id = ctx
        .db
        .insert_mod(forge_mod_id, forge_version_id, name, slug, version)?;

    for file in &extracted_files {
        ctx.db
            .insert_file(db_id, &file.path, Some(&file.hash), Some(file.size as i64))?;
    }

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
        if db.get_mod_by_forge_id(node.mod_id)?.is_some() {
            continue;
        }
        if out.iter().any(|p| p.mod_id == node.mod_id) {
            continue;
        }

        // Recurse into children first so deps install before their parents
        if let Some(ref children) = node.resolved_dependencies {
            collect_deps_to_install(children, db, out)?;
        }

        out.push(PendingInstall {
            mod_id: node.mod_id,
            version_id: node.version_id,
            name: node
                .name
                .clone()
                .unwrap_or_else(|| format!("mod-{}", node.mod_id)),
            version: node
                .version
                .clone()
                .unwrap_or_else(|| "unknown".to_string()),
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
        let db = Database::open_in_memory().unwrap();
        db.insert_mod(10, 20, "AlreadyInstalled", None, "1.0.0")
            .unwrap();

        let nodes = vec![DependencyNode {
            mod_id: 10,
            version_id: 20,
            name: Some("AlreadyInstalled".to_string()),
            version: Some("1.0.0".to_string()),
            resolved_dependencies: None,
        }];

        let mut out = Vec::new();
        collect_deps_to_install(&nodes, &db, &mut out).unwrap();

        assert!(out.is_empty(), "should skip already-installed deps");
    }

    #[test]
    fn collect_deps_flattens_tree_children_first() {
        let db = Database::open_in_memory().unwrap();

        let nodes = vec![DependencyNode {
            mod_id: 10,
            version_id: 20,
            name: Some("Parent".to_string()),
            version: Some("1.0.0".to_string()),
            resolved_dependencies: Some(vec![DependencyNode {
                mod_id: 30,
                version_id: 40,
                name: Some("Child".to_string()),
                version: Some("0.5.0".to_string()),
                resolved_dependencies: None,
            }]),
        }];

        let mut out = Vec::new();
        collect_deps_to_install(&nodes, &db, &mut out).unwrap();

        assert_eq!(out.len(), 2);
        assert_eq!(out[0].mod_id, 30); // Child first (install order)
        assert_eq!(out[1].mod_id, 10); // Parent second
    }

    #[test]
    fn collect_deps_deduplicates() {
        let db = Database::open_in_memory().unwrap();

        let shared_dep = DependencyNode {
            mod_id: 99,
            version_id: 100,
            name: Some("SharedLib".to_string()),
            version: Some("1.0.0".to_string()),
            resolved_dependencies: None,
        };

        let nodes = vec![
            DependencyNode {
                mod_id: 10,
                version_id: 20,
                name: Some("ModA".to_string()),
                version: Some("1.0.0".to_string()),
                resolved_dependencies: Some(vec![shared_dep.clone()]),
            },
            DependencyNode {
                mod_id: 30,
                version_id: 40,
                name: Some("ModB".to_string()),
                version: Some("2.0.0".to_string()),
                resolved_dependencies: Some(vec![DependencyNode {
                    mod_id: 99,
                    version_id: 100,
                    name: Some("SharedLib".to_string()),
                    version: Some("1.0.0".to_string()),
                    resolved_dependencies: None,
                }]),
            },
        ];

        let mut out = Vec::new();
        collect_deps_to_install(&nodes, &db, &mut out).unwrap();

        let shared_count = out.iter().filter(|p| p.mod_id == 99).count();
        assert_eq!(shared_count, 1, "SharedLib should appear only once");
    }
}
