use anyhow::Result;

use super::common::{confirm, CliContext};

pub async fn run(
    backup_id: Option<i64>,
    latest: Option<&str>,
    force: bool,
    ctx: &CliContext,
) -> Result<()> {
    // Check server is not running
    let running =
        crate::server_detect::is_server_running(&ctx.config, &ctx.dirs, ctx.container_mgr.as_ref())
            .await?;
    if running {
        anyhow::bail!("Server is running — stop it before restoring a backup.");
    }

    let backup = match (backup_id, latest) {
        (Some(id), _) => ctx
            .db
            .get_backup(id)?
            .ok_or_else(|| anyhow::anyhow!("backup #{id} not found"))?,
        (None, Some(mod_ref)) => {
            let installed = super::common::resolve_installed_mod(mod_ref, ctx)?;
            let forge_mod_id = installed.forge_mod_id.ok_or_else(|| {
                anyhow::anyhow!("backups are not supported for mods installed from URLs or files")
            })?;
            ctx.db
                .get_latest_backup_for_mod(forge_mod_id)?
                .ok_or_else(|| anyhow::anyhow!("no backups found for {}", installed.name))?
        }
        (None, None) => anyhow::bail!("specify a backup ID or use --latest <mod>"),
    };

    println!(
        "Backup #{}: {} {} ({})",
        backup.id,
        backup.mod_name.as_deref().unwrap_or("full snapshot"),
        backup.mod_version.as_deref().unwrap_or(""),
        backup.created_at
    );

    if !force && !confirm("Restore this backup? Current files will be overwritten.")? {
        println!("Restore cancelled.");
        return Ok(());
    }

    match backup.backup_type.as_str() {
        "mod" => {
            crate::backup::restore_mod_backup(&ctx.db, &ctx.dirs, &ctx.config, backup.id)?;
            println!(
                "Restored {} to v{}",
                backup.mod_name.as_deref().unwrap_or("mod"),
                backup.mod_version.as_deref().unwrap_or("?")
            );
        }
        "full" => {
            crate::backup::restore_full_backup(&ctx.db, &ctx.dirs, &ctx.config, backup.id)?;
            println!("Full backup restored. Restart the web server to reload config.");
        }
        other => anyhow::bail!("unknown backup type: {other}"),
    }
    Ok(())
}
