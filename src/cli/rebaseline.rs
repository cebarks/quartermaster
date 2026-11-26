use anyhow::{bail, ensure, Result};
use std::collections::BTreeSet;

use super::common::CliContext;

/// Rebaseline only explicitly selected legacy records after an acknowledgement.
pub fn run(file_ids: &[i64], trust_current: bool, ctx: &CliContext) -> Result<()> {
    ensure!(
        trust_current,
        "explicit --trust-current acknowledgement is required"
    );
    let selected: BTreeSet<_> = file_ids.iter().copied().collect();
    ensure!(!selected.is_empty(), "select at least one --file-id");
    let pending = ctx.db.pending_file_rehashes()?;
    for id in &selected {
        ensure!(
            pending.iter().any(|(file, _)| file.id == *id),
            "file ID {id} is not a pending legacy record"
        );
    }
    eprintln!("Warning: trusting current file bytes as new XXH3-128 baselines; legacy checksums are not verification.");
    for (file, _) in pending
        .iter()
        .filter(|(file, _)| selected.contains(&file.id))
    {
        eprintln!("  #{} {}", file.id, file.file_path);
    }
    let count = crate::ops::rebaseline_selected_hashes(&ctx.db, &ctx.dirs, file_ids)?;
    if count != selected.len() {
        bail!(
            "rebaselined {count} of {} files; unavailable or changed records remain pending",
            selected.len()
        );
    }
    println!("Rebaselined {count} selected file(s).");
    Ok(())
}
