use std::path::Path;

use anyhow::{bail, Context, Result};

use crate::spt::mods::compute_file_hash;

use super::common::{resolve_mod, CliContext};

pub async fn run(path: &str, forge_mod_ref: &str, ctx: &CliContext) -> Result<()> {
    // 1. Validate the path exists under the SPT root
    let full_path = ctx.spt_dir.join(path);
    if !full_path.exists() {
        bail!("path does not exist: {}", full_path.display());
    }
    if !full_path.is_dir() {
        bail!("path is not a directory: {}", full_path.display());
    }

    // Ensure the path is under SPT/user/mods/ or BepInEx/plugins/
    if !path.starts_with("SPT/user/mods/") && !path.starts_with("BepInEx/plugins/") {
        bail!(
            "path must be under SPT/user/mods/ or BepInEx/plugins/, got: {}",
            path
        );
    }

    // 2. Resolve the Forge mod
    let forge_mod = resolve_mod(&ctx.forge, forge_mod_ref).await?;
    println!("Forge mod: {} (ID: {})", forge_mod.name, forge_mod.id);

    // Check if already tracked
    if let Some(existing) = ctx.db.get_mod_by_forge_id(forge_mod.id)? {
        bail!(
            "{} is already tracked (version {})",
            existing.name,
            existing.version
        );
    }

    // 3. Determine version — get latest versions and try to match, or use "unknown"
    let versions = ctx
        .forge
        .get_versions(forge_mod.id, Some(&ctx.spt_info.spt_version))
        .await?;

    let (version_id, version_str) = if let Some(latest) = versions.first() {
        println!(
            "Assuming version: {} (latest compatible with SPT {})",
            latest.version, ctx.spt_info.spt_version
        );
        (latest.id, latest.version.clone())
    } else {
        // Fall back to any version
        let all_versions = ctx.forge.get_versions(forge_mod.id, None).await?;
        if let Some(latest) = all_versions.first() {
            println!(
                "Warning: no SPT {}-compatible version found. Using latest: {}",
                ctx.spt_info.spt_version, latest.version
            );
            (latest.id, latest.version.clone())
        } else {
            bail!("no versions found for {} on Forge", forge_mod.name);
        }
    };

    // 4. Scan directory for files
    let mut files = Vec::new();
    scan_dir_for_tracking(&full_path, &ctx.spt_dir, &mut files)?;

    if files.is_empty() {
        bail!("no files found in {}", path);
    }

    println!("Found {} files to track", files.len());

    // 5. Record in database
    let db_id = ctx.db.insert_mod(
        forge_mod.id,
        version_id,
        &forge_mod.name,
        forge_mod.slug.as_deref(),
        &version_str,
    )?;

    for (rel_path, hash, size) in &files {
        ctx.db
            .insert_file(db_id, rel_path, Some(hash.as_str()), Some(*size as i64))?;
    }

    println!(
        "\n{} v{} is now tracked ({} files).",
        forge_mod.name,
        version_str,
        files.len()
    );

    Ok(())
}

/// Recursively scan a directory, collecting (relative_path, sha256_hash, size) for each file.
fn scan_dir_for_tracking(
    dir: &Path,
    spt_root: &Path,
    out: &mut Vec<(String, String, u64)>,
) -> Result<()> {
    let entries = std::fs::read_dir(dir)
        .with_context(|| format!("failed to read directory: {}", dir.display()))?;

    for entry in entries {
        let entry = entry?;
        let path = entry.path();

        if path.is_dir() {
            scan_dir_for_tracking(&path, spt_root, out)?;
        } else {
            let rel = path
                .strip_prefix(spt_root)
                .with_context(|| "path not under SPT root")?
                .to_string_lossy()
                .to_string();

            let hash = compute_file_hash(&path)?;
            let size = std::fs::metadata(&path)?.len();

            out.push((rel, hash, size));
        }
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[test]
    fn scan_dir_collects_files() {
        let tmp = TempDir::new().unwrap();
        let root = tmp.path();

        let mod_dir = root.join("SPT/user/mods/TestMod");
        std::fs::create_dir_all(&mod_dir).unwrap();
        std::fs::write(mod_dir.join("package.json"), b"{}").unwrap();
        std::fs::write(mod_dir.join("mod.ts"), b"// code").unwrap();

        let mut files = Vec::new();
        scan_dir_for_tracking(&mod_dir, root, &mut files).unwrap();

        assert_eq!(files.len(), 2);
        assert!(files
            .iter()
            .any(|(p, _, _)| p == "SPT/user/mods/TestMod/package.json"));
        assert!(files
            .iter()
            .any(|(p, _, _)| p == "SPT/user/mods/TestMod/mod.ts"));

        // Verify hashes are present
        for (_, hash, _) in &files {
            assert_eq!(hash.len(), 64);
        }
    }

    #[test]
    fn scan_dir_empty() {
        let tmp = TempDir::new().unwrap();
        let dir = tmp.path().join("empty");
        std::fs::create_dir_all(&dir).unwrap();

        let mut files = Vec::new();
        scan_dir_for_tracking(&dir, tmp.path(), &mut files).unwrap();

        assert!(files.is_empty());
    }
}
