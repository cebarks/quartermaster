use anyhow::Result;

use super::common::CliContext;

pub fn run(mod_ref: Option<&str>, list: bool, ctx: &CliContext) -> Result<()> {
    if list {
        return list_backups(mod_ref, ctx);
    }

    match mod_ref {
        Some(r) => {
            let installed = super::common::resolve_installed_mod(r, ctx)?;
            let backup_id = crate::backup::backup_mod(
                &ctx.db,
                &ctx.spt_dir,
                &ctx.config,
                installed.id,
                "manual",
            )?;
            let backup = ctx
                .db
                .get_backup(backup_id)?
                .ok_or_else(|| anyhow::anyhow!("backup record not found after creation"))?;
            println!(
                "Backed up {} v{} (backup #{})",
                installed.name, installed.version, backup.id
            );
        }
        None => {
            let backup_id = crate::backup::backup_full(&ctx.db, &ctx.spt_dir, &ctx.config)?;
            let backup = ctx
                .db
                .get_backup(backup_id)?
                .ok_or_else(|| anyhow::anyhow!("backup record not found after creation"))?;
            let mods = ctx.db.list_mods()?;
            println!(
                "Full backup created (backup #{}, {} mods)",
                backup.id,
                mods.len()
            );
        }
    }
    Ok(())
}

fn list_backups(mod_ref: Option<&str>, ctx: &CliContext) -> Result<()> {
    let backups = match mod_ref {
        Some(r) => {
            let installed = super::common::resolve_installed_mod(r, ctx)?;
            ctx.db.list_backups_for_mod(installed.forge_mod_id)?
        }
        None => ctx.db.list_all_backups()?,
    };

    if backups.is_empty() {
        println!("No backups found.");
        return Ok(());
    }

    println!(
        "{:<6} {:<6} {:<20} {:<12} {:<14} {:<10}",
        "ID", "Type", "Mod", "Version", "Trigger", "Date"
    );
    println!("{}", "-".repeat(70));

    for b in &backups {
        println!(
            "{:<6} {:<6} {:<20} {:<12} {:<14} {:<10}",
            b.id,
            b.backup_type,
            b.mod_name.as_deref().unwrap_or("-"),
            b.mod_version.as_deref().unwrap_or("-"),
            b.trigger,
            b.created_at.get(..10).unwrap_or(&b.created_at),
        );
    }
    Ok(())
}
