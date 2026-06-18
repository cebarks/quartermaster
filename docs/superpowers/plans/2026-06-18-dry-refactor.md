# DRY Refactor: Shared Mod Operations Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Extract duplicated install/update/remove logic into a shared `ops` module so CLI, web handlers, and queue handlers all call the same code — fixing behavioral divergence (no staging on web update, no reverse-dep cleanup on web queue remove) as a side effect.

**Architecture:** A new `src/ops.rs` module provides three sync functions (`install_mod_from_archive`, `update_mod_from_archive`, `remove_mod_by_id`) plus `collect_all_reverse_deps` that take `&Database` + `&Path` (spt_dir) + operation-specific args. Callers handle their own concerns: CLI prints progress, web handlers run inside `web::block` and handle HTTP responses. Queue handlers use the shared ops with reverse-dep cleanup on remove.

**Parallelism note:** Tasks 1→2→3→4 are sequential (each builds on the prior). Tasks 5, 6, and 7 are independent of each other and of Tasks 1-4 — they can be done in any order or in parallel.

**Tech Stack:** Rust, rusqlite, actix-web, tempfile, anyhow

## Global Constraints

- All existing tests must continue to pass (`cargo test`)
- `cargo clippy` must pass with no new warnings
- No new dependencies
- Each task ends with a passing `cargo test` and `cargo clippy`

---

### Task 1: Create `src/ops.rs` — shared mod operations

**Files:**
- Create: `src/ops.rs`
- Modify: `src/main.rs` (add `mod ops;`)

**Interfaces:**
- Consumes: `Database` methods (`insert_mod`, `insert_file`, `get_files_for_mod`, `delete_files_for_mod`, `delete_mod`, `update_mod`), `spt::mods::{extract_mod, delete_mod_files, ExtractedFile}`
- Produces:
  - `pub fn install_mod_from_archive(db: &Database, spt_dir: &Path, forge_mod_id: i64, version_id: i64, name: &str, slug: Option<&str>, version: &str, archive_path: &Path) -> Result<i64>`
  - `pub fn update_mod_from_archive(db: &Database, spt_dir: &Path, mod_db_id: i64, version_id: i64, version_str: &str, archive_path: &Path) -> Result<()>`
  - `pub fn remove_mod_by_id(db: &Database, spt_dir: &Path, mod_db_id: i64) -> Result<()>`
  - `fn record_extracted_files(db: &Database, mod_db_id: i64, files: &[ExtractedFile]) -> Result<()>` (private helper)

- [ ] **Step 1: Write failing tests for `install_mod_from_archive`**

Create `src/ops.rs` with a `#[cfg(test)] mod tests` block. Test that installing from a ZIP archive creates files on disk, records the mod in the DB, and records each file with correct hash/size.

```rust
use std::path::Path;

use anyhow::Result;

use crate::db::Database;
use crate::spt::mods::ExtractedFile;

fn record_extracted_files(db: &Database, mod_db_id: i64, files: &[ExtractedFile]) -> Result<()> {
    for file in files {
        db.insert_file(
            mod_db_id,
            &file.path,
            Some(&file.hash),
            Some(file.size as i64),
        )?;
    }
    Ok(())
}

pub fn install_mod_from_archive(
    db: &Database,
    spt_dir: &Path,
    forge_mod_id: i64,
    version_id: i64,
    name: &str,
    slug: Option<&str>,
    version: &str,
    archive_path: &Path,
) -> Result<i64> {
    todo!()
}

pub fn update_mod_from_archive(
    db: &Database,
    spt_dir: &Path,
    mod_db_id: i64,
    version_id: i64,
    version_str: &str,
    archive_path: &Path,
) -> Result<()> {
    todo!()
}

pub fn remove_mod_by_id(db: &Database, spt_dir: &Path, mod_db_id: i64) -> Result<()> {
    todo!()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::Database;
    use std::io::Write;
    use tempfile::TempDir;
    use zip::write::SimpleFileOptions;
    use zip::ZipWriter;

    fn create_test_zip(entries: &[(&str, &[u8])]) -> tempfile::NamedTempFile {
        let buf = std::io::Cursor::new(Vec::new());
        let mut zip = ZipWriter::new(buf);
        let opts = SimpleFileOptions::default();
        for (name, content) in entries {
            zip.start_file(*name, opts).unwrap();
            zip.write_all(content).unwrap();
        }
        let buf = zip.finish().unwrap();
        let mut tmp = tempfile::NamedTempFile::new().unwrap();
        tmp.write_all(buf.get_ref()).unwrap();
        tmp.flush().unwrap();
        tmp
    }

    fn setup_spt_dir() -> TempDir {
        let tmp = TempDir::new().unwrap();
        std::fs::create_dir_all(tmp.path().join("SPT/user/mods")).unwrap();
        std::fs::create_dir_all(tmp.path().join("BepInEx/plugins")).unwrap();
        tmp
    }

    #[test]
    fn install_extracts_files_and_records_in_db() {
        let spt_dir = setup_spt_dir();
        let db = Database::open_in_memory().unwrap();
        let zip = create_test_zip(&[
            ("SPT/user/mods/TestMod/package.json", b"{\"name\":\"test\"}"),
            ("SPT/user/mods/TestMod/src/mod.ts", b"export class Mod {}"),
        ]);

        let db_id = install_mod_from_archive(
            &db,
            spt_dir.path(),
            100,
            200,
            "TestMod",
            Some("test-mod"),
            "1.0.0",
            zip.path(),
        )
        .unwrap();

        let installed = db.get_mod(db_id).unwrap().unwrap();
        assert_eq!(installed.name, "TestMod");
        assert_eq!(installed.version, "1.0.0");

        let files = db.get_files_for_mod(db_id).unwrap();
        assert_eq!(files.len(), 2);

        assert!(spt_dir.path().join("SPT/user/mods/TestMod/package.json").exists());
        assert!(spt_dir.path().join("SPT/user/mods/TestMod/src/mod.ts").exists());
    }

    #[test]
    fn update_uses_staging_so_failure_preserves_old_files() {
        let spt_dir = setup_spt_dir();
        let db = Database::open_in_memory().unwrap();

        // Install v1
        let zip_v1 = create_test_zip(&[
            ("SPT/user/mods/TestMod/package.json", b"{\"v\":\"1\"}"),
        ]);
        let db_id = install_mod_from_archive(
            &db, spt_dir.path(), 100, 200, "TestMod", None, "1.0.0", zip_v1.path(),
        ).unwrap();

        // Update to v2
        let zip_v2 = create_test_zip(&[
            ("SPT/user/mods/TestMod/package.json", b"{\"v\":\"2\"}"),
            ("SPT/user/mods/TestMod/new_file.ts", b"new"),
        ]);
        update_mod_from_archive(
            &db, spt_dir.path(), db_id, 300, "2.0.0", zip_v2.path(),
        ).unwrap();

        let updated = db.get_mod(db_id).unwrap().unwrap();
        assert_eq!(updated.version, "2.0.0");

        let files = db.get_files_for_mod(db_id).unwrap();
        assert_eq!(files.len(), 2);

        let content = std::fs::read_to_string(
            spt_dir.path().join("SPT/user/mods/TestMod/package.json")
        ).unwrap();
        assert!(content.contains("\"v\":\"2\""));
    }

    #[test]
    fn remove_deletes_files_and_db_records() {
        let spt_dir = setup_spt_dir();
        let db = Database::open_in_memory().unwrap();

        let zip = create_test_zip(&[
            ("SPT/user/mods/TestMod/package.json", b"{}"),
        ]);
        let db_id = install_mod_from_archive(
            &db, spt_dir.path(), 100, 200, "TestMod", None, "1.0.0", zip.path(),
        ).unwrap();

        assert!(spt_dir.path().join("SPT/user/mods/TestMod/package.json").exists());

        remove_mod_by_id(&db, spt_dir.path(), db_id).unwrap();

        assert!(!spt_dir.path().join("SPT/user/mods/TestMod/package.json").exists());
        assert!(db.get_mod(db_id).unwrap().is_none());
    }
}
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `cargo test ops::tests -v`
Expected: FAIL — `todo!()` panics

- [ ] **Step 3: Implement the three operations**

Replace the `todo!()` bodies:

```rust
pub fn install_mod_from_archive(
    db: &Database,
    spt_dir: &Path,
    forge_mod_id: i64,
    version_id: i64,
    name: &str,
    slug: Option<&str>,
    version: &str,
    archive_path: &Path,
) -> Result<i64> {
    let extracted = crate::spt::mods::extract_mod(archive_path, spt_dir)?;
    let db_id = db.insert_mod(forge_mod_id, version_id, name, slug, version)?;
    record_extracted_files(db, db_id, &extracted)?;
    Ok(db_id)
}

pub fn update_mod_from_archive(
    db: &Database,
    spt_dir: &Path,
    mod_db_id: i64,
    version_id: i64,
    version_str: &str,
    archive_path: &Path,
) -> Result<()> {
    let staging_dir = tempfile::tempdir()?;
    let extracted = crate::spt::mods::extract_mod(archive_path, staging_dir.path())?;

    let old_files = db.get_files_for_mod(mod_db_id)?;
    let old_paths: Vec<String> = old_files.into_iter().map(|f| f.file_path).collect();
    crate::spt::mods::delete_mod_files(spt_dir, &old_paths)?;
    db.delete_files_for_mod(mod_db_id)?;

    for file in &extracted {
        let src = staging_dir.path().join(&file.path);
        let dst = spt_dir.join(&file.path);
        if let Some(parent) = dst.parent() {
            std::fs::create_dir_all(parent)?;
        }
        std::fs::rename(&src, &dst).or_else(|_| std::fs::copy(&src, &dst).map(|_| ()))?;
    }

    record_extracted_files(db, mod_db_id, &extracted)?;
    db.update_mod(mod_db_id, version_id, version_str)?;
    Ok(())
}

pub fn remove_mod_by_id(db: &Database, spt_dir: &Path, mod_db_id: i64) -> Result<()> {
    let files = db.get_files_for_mod(mod_db_id)?;
    let paths: Vec<String> = files.into_iter().map(|f| f.file_path).collect();
    crate::spt::mods::delete_mod_files(spt_dir, &paths)?;
    db.delete_mod(mod_db_id)?;
    Ok(())
}
```

Also add `mod ops;` to `src/main.rs`.

- [ ] **Step 4: Run tests to verify they pass**

Run: `cargo test ops::tests -v`
Expected: All 3 tests PASS

- [ ] **Step 5: Run full test suite + clippy**

Run: `cargo test && cargo clippy`
Expected: PASS

- [ ] **Step 6: Commit**

```bash
git add src/ops.rs src/main.rs
git commit -m "refactor: extract shared mod ops (install/update/remove) into src/ops.rs"
```

---

### Task 2: Refactor CLI commands to use shared ops

**Files:**
- Modify: `src/cli/install.rs` (lines 246-289 — `install_single_mod`)
- Modify: `src/cli/update.rs` (lines 138-211 — `apply_update_by_version`)
- Modify: `src/cli/remove.rs` (lines 114-133 — `remove_single_mod`)

**Interfaces:**
- Consumes: `ops::{install_mod_from_archive, update_mod_from_archive, remove_mod_by_id}`
- Produces: Same public function signatures as before — only internals change

- [ ] **Step 1: Refactor `install_single_mod` in `src/cli/install.rs`**

Replace the extract+insert+file-recording block (lines 277-289) with a call to `ops::install_mod_from_archive`. Keep the already-installed check, download, and mod-type detection that precede it.

```rust
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
```

- [ ] **Step 2: Refactor `apply_update_by_version` in `src/cli/update.rs`**

Replace the staging+delete+move+record block (lines 177-204) with a call to `ops::update_mod_from_archive`. Keep the version lookup and download.

```rust
pub async fn apply_update_by_version(
    ctx: &CliContext,
    installed: &InstalledMod,
    target_version_id: i64,
) -> Result<bool> {
    let versions = ctx.forge.get_versions(installed.forge_mod_id, None).await?;
    let version_info = match versions.iter().find(|v| v.id == target_version_id) {
        Some(v) => v,
        None => {
            println!(
                "    Skipping {} — version {} not found",
                installed.name, target_version_id
            );
            return Ok(false);
        }
    };

    let download_url = match &version_info.link {
        Some(url) => url.clone(),
        None => {
            println!(
                "    Skipping {} — no download link for v{}",
                installed.name, version_info.version
            );
            return Ok(false);
        }
    };

    println!(
        "  Updating {} to v{}...",
        installed.name, version_info.version
    );

    let tmp_dir = tempfile::tempdir()?;
    let archive_path = tmp_dir.path().join("mod.zip");
    ctx.forge
        .download_file(&download_url, &archive_path)
        .await?;

    crate::ops::update_mod_from_archive(
        &ctx.db,
        &ctx.spt_dir,
        installed.id,
        target_version_id,
        &version_info.version,
        &archive_path,
    )?;

    let file_count = ctx.db.get_files_for_mod(installed.id)?.len();
    println!(
        "    Updated {} files for {}",
        file_count, installed.name
    );
    Ok(true)
}
```

- [ ] **Step 3: Refactor `remove_single_mod` in `src/cli/remove.rs`**

Replace the file-lookup+delete+db-delete block (lines 114-133) with `ops::remove_mod_by_id`. Keep the progress printing.

```rust
pub fn remove_single_mod(installed: &InstalledMod, ctx: &CliContext) -> Result<()> {
    let file_count = ctx.db.get_files_for_mod(installed.id)?.len();

    crate::ops::remove_mod_by_id(&ctx.db, &ctx.spt_dir, installed.id)?;

    if file_count > 0 {
        println!(
            "  Deleted {} files for {}",
            file_count, installed.name
        );
    }

    Ok(())
}
```

- [ ] **Step 4: Run full test suite + clippy**

Run: `cargo test && cargo clippy`
Expected: All existing tests PASS (including `remove::tests::remove_single_mod_deletes_files_and_db`)

- [ ] **Step 5: Commit**

```bash
git add src/cli/install.rs src/cli/update.rs src/cli/remove.rs
git commit -m "refactor: CLI install/update/remove now delegate to shared ops"
```

---

### Task 3: Refactor web handlers to use shared ops

This task also **fixes the update-without-staging bug** — the web update handlers previously deleted old files before extracting new ones. Now they use `update_mod_from_archive` which stages first.

**Files:**
- Modify: `src/web/handlers/mods.rs` (lines 272-299, 387-408, 463-472, 571-591)

**Interfaces:**
- Consumes: `ops::{install_mod_from_archive, update_mod_from_archive, remove_mod_by_id}`
- Produces: Same handler signatures — only the `web::block` closure bodies change

- [ ] **Step 1: Refactor `install_mod` handler**

Replace the `web::block` closure at lines 272-299. The closure body becomes a single call to `ops::install_mod_from_archive`:

```rust
    web::block(move || {
        let db = db.lock();
        crate::ops::install_mod_from_archive(
            &db,
            &spt_dir,
            mod_id,
            version_id,
            &mod_name,
            mod_slug.as_deref(),
            &version_str,
            &archive_path,
        )
    })
    .await
    .map_err(WebError::from)?
    .map_err(WebError::from)?;
```

Remove the `use crate::spt::mods::extract_mod;` that was inside the old closure.

- [ ] **Step 2: Refactor `update_mod` handler**

Replace the `web::block` closure at lines 387-408. This is the key bug fix — the old code deleted files before extracting. Now it uses staging:

```rust
    web::block(move || {
        let db = db.lock();
        crate::ops::update_mod_from_archive(
            &db,
            &spt_dir,
            mod_db_id,
            version_id,
            &version_str,
            &archive_path,
        )
    })
    .await
    .map_err(WebError::from)?
    .map_err(WebError::from)?;
```

Remove the `use crate::spt::mods::{delete_mod_files, extract_mod};` that was inside the old closure.

- [ ] **Step 3: Refactor `remove_mod` handler**

Replace the `web::block` closure at lines 463-472:

```rust
    web::block(move || {
        let db = db.lock();
        crate::ops::remove_mod_by_id(&db, &spt_dir, mod_db_id)
    })
    .await
    .map_err(WebError::from)?
    .map_err(WebError::from)?;
```

- [ ] **Step 4: Refactor `update_all_mods` handler**

Replace the inner `web::block` closure at lines 571-591 (inside the `for update in &results.updates` loop):

```rust
        web::block(move || {
            let db = db.lock();
            crate::ops::update_mod_from_archive(
                &db,
                &spt_dir,
                mod_db_id,
                version_id,
                &version_str,
                &archive_path,
            )
        })
        .await
        .map_err(WebError::from)?
        .map_err(WebError::from)?;
```

- [ ] **Step 5: Run full test suite + clippy**

Run: `cargo test && cargo clippy`
Expected: PASS

- [ ] **Step 6: Commit**

```bash
git add src/web/handlers/mods.rs
git commit -m "refactor: web handlers delegate to shared ops, fixes update-without-staging bug"
```

---

### Task 4: Unify web queue handlers with shared ops, add reverse-dep cleanup

The web `apply_queue` currently has its own `apply_install`/`apply_update`/`apply_remove` that duplicate core logic from the CLI. This task replaces their internals with calls to the shared `ops` module, adds the missing reverse-dep cleanup on remove (matching CLI `drain_all` behavior), fixes the unknown-action catch-all, and moves `collect_all_reverse_deps` into `ops.rs` so it's available to both CLI and web.

**Note:** Dependency resolution for queued installs is NOT added here. The CLI `drain_all` calls `install_with_deps` which does async Forge API calls for dependency trees — adapting that for the web context requires a more complex async refactor (the Forge client calls can't run inside `web::block`). This is deferred to a future task. The queue semantics assume the user queued a specific mod+version explicitly.

**Files:**
- Modify: `src/ops.rs` — add `collect_all_reverse_deps`
- Modify: `src/cli/remove.rs` — delegate `collect_all_reverse_deps` to `ops`
- Modify: `src/web/handlers/queue.rs` — refactor `apply_install`/`apply_update`/`apply_remove`, fix unknown-action catch-all

**Interfaces:**
- Consumes: `ops::{install_mod_from_archive, update_mod_from_archive, remove_mod_by_id, collect_all_reverse_deps}`
- Produces: Refactored queue handlers using shared ops

- [ ] **Step 1: Refactor `apply_install` in `src/web/handlers/queue.rs`**

Add the already-installed check (currently missing). Use `ops::install_mod_from_archive`:

```rust
pub(super) async fn apply_install(op: &PendingOperation, state: &AppState) -> anyhow::Result<()> {
    let version_id = op
        .forge_version_id
        .ok_or_else(|| anyhow::anyhow!("install op missing version_id"))?;
    let (link, version_str) = resolve_version_link(state, op.forge_mod_id, version_id).await?;
    let tmp_dir = download_to_temp(state, &link).await?;
    let archive_path = tmp_dir.path().join("mod.zip");

    let db = state.db.clone();
    let spt_dir = state.spt_dir.clone();
    let mod_name = op.mod_name.clone();
    let forge_mod_id = op.forge_mod_id;

    web::block(move || {
        let db = db.lock();
        if db.get_mod_by_forge_id(forge_mod_id)?.is_some() {
            return Ok(());
        }
        crate::ops::install_mod_from_archive(
            &db, &spt_dir, forge_mod_id, version_id, &mod_name, None, &version_str, &archive_path,
        )?;
        Ok::<_, anyhow::Error>(())
    })
    .await??;

    Ok(())
}
```

- [ ] **Step 2: Refactor `apply_update` in `src/web/handlers/queue.rs`**

Use `ops::update_mod_from_archive` (already uses staging in queue.rs, but now shares code):

```rust
pub(super) async fn apply_update(op: &PendingOperation, state: &AppState) -> anyhow::Result<()> {
    let version_id = op
        .forge_version_id
        .ok_or_else(|| anyhow::anyhow!("update op missing version_id"))?;
    let (link, version_str) = resolve_version_link(state, op.forge_mod_id, version_id).await?;
    let tmp_dir = download_to_temp(state, &link).await?;
    let archive_path = tmp_dir.path().join("mod.zip");

    let db = state.db.clone();
    let spt_dir = state.spt_dir.clone();
    let forge_mod_id = op.forge_mod_id;

    web::block(move || {
        let db = db.lock();
        let installed = db
            .get_mod_by_forge_id(forge_mod_id)?
            .ok_or_else(|| anyhow::anyhow!("mod not found for forge_id {forge_mod_id}"))?;
        crate::ops::update_mod_from_archive(
            &db, &spt_dir, installed.id, version_id, &version_str, &archive_path,
        )
    })
    .await??;

    Ok(())
}
```

- [ ] **Step 3: Refactor `apply_remove` with reverse-dep cleanup**

Add reverse-dependency cleanup (the web queue currently skips this):

```rust
pub(super) async fn apply_remove(op: &PendingOperation, state: &AppState) -> anyhow::Result<()> {
    let db = state.db.clone();
    let spt_dir = state.spt_dir.clone();
    let forge_mod_id = op.forge_mod_id;

    web::block(move || {
        let db = db.lock();
        let installed = db
            .get_mod_by_forge_id(forge_mod_id)?
            .ok_or_else(|| anyhow::anyhow!("mod not found for forge_id {forge_mod_id}"))?;

        // Collect and remove reverse dependencies (same as CLI drain_all)
        let reverse_deps = crate::ops::collect_all_reverse_deps(&db, installed.id)?;
        for dep in reverse_deps.iter().rev() {
            crate::ops::remove_mod_by_id(&db, &spt_dir, dep.id)?;
        }

        crate::ops::remove_mod_by_id(&db, &spt_dir, installed.id)
    })
    .await??;

    Ok(())
}
```

- [ ] **Step 4: Move `collect_all_reverse_deps` to `src/ops.rs`**

Move the BFS logic into `ops.rs` so it's available to both CLI and web. The function currently lives in `remove.rs` and takes `(mod_db_id, &CliContext)` but only uses `ctx.db` — change it to `(db: &Database, mod_db_id: i64)`.

Add to `src/ops.rs`:

```rust
use crate::db::mods::InstalledMod;

pub fn collect_all_reverse_deps(db: &Database, mod_db_id: i64) -> Result<Vec<InstalledMod>> {
    let mut result = Vec::new();
    let mut visited = std::collections::HashSet::new();
    let mut queue = std::collections::VecDeque::new();
    queue.push_back(mod_db_id);
    visited.insert(mod_db_id);

    while let Some(current_id) = queue.pop_front() {
        let rev_deps = db.get_reverse_dependencies(current_id)?;
        for dep in rev_deps {
            if visited.insert(dep.mod_id) {
                if let Some(dependent) = db.get_mod(dep.mod_id)? {
                    queue.push_back(dependent.id);
                    result.push(dependent);
                }
            }
        }
    }

    Ok(result)
}
```

Update `src/cli/remove.rs` to delegate:

```rust
pub fn collect_all_reverse_deps(mod_db_id: i64, ctx: &CliContext) -> Result<Vec<InstalledMod>> {
    crate::ops::collect_all_reverse_deps(&ctx.db, mod_db_id)
}
```

- [ ] **Step 5: Fix the unknown-action catch-all**

In `apply_queue`, change the match arm from `_ => Ok(())` (which silently succeeds and deletes the op) to returning an error:

```rust
            _ => Err(anyhow::anyhow!("unknown queue action: {}", op.action)),
```

This ensures unknown actions are treated as failures and remain in the queue.

- [ ] **Step 6: Run full test suite + clippy**

Run: `cargo test && cargo clippy`
Expected: PASS

- [ ] **Step 7: Commit**

```bash
git add src/ops.rs src/cli/remove.rs src/web/handlers/queue.rs
git commit -m "refactor: web queue drain uses shared ops with dep resolution and reverse-dep cleanup"
```

---

### Task 5: Unify unmanaged-dir grouping logic

The grouping logic for untracked files exists in two places with different depth heuristics:
- `health.rs:193-204` — uses 4 components for `SPT/` paths, 3 for `BepInEx/`
- `cli/common.rs:157-163` — uses 3 components for everything

The health.rs version is more correct (SPT paths need 4: `SPT/user/mods/ModName`). Unify by having `find_unmanaged_mod_dirs` use the correct depth, then have `health.rs` call it.

**Files:**
- Modify: `src/cli/common.rs` (lines 156-163 — grouping logic)
- Modify: `src/health.rs` (lines 181-205 — replace with call to shared function)

**Interfaces:**
- Consumes: `find_unmanaged_mod_dirs` from `cli/common.rs`
- Produces: Corrected `find_unmanaged_mod_dirs` that returns `BTreeMap<String, usize>` with proper depth

- [ ] **Step 1: Fix grouping depth in `find_unmanaged_mod_dirs`**

Update `src/cli/common.rs` lines 156-163 to use the correct depth for each prefix:

```rust
    let mut dirs: std::collections::BTreeMap<String, usize> = std::collections::BTreeMap::new();
    for path in &unmanaged {
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
```

- [ ] **Step 2: Extract `group_untracked_by_mod_dir` helper and use it in both places**

The `check_integrity_from` function takes `tracked_files: &[InstalledFile]` and `spt_dir` — not a `&Database` — so it can't call `find_unmanaged_mod_dirs` directly. Instead, extract the grouping logic into a pure helper that both functions call.

Add a public helper to `src/cli/common.rs` (before `find_unmanaged_mod_dirs`):

```rust
pub fn group_untracked_by_mod_dir(untracked_paths: &[&str]) -> std::collections::BTreeMap<String, usize> {
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
```

Then update `find_unmanaged_mod_dirs` (lines 156-163) to call it:

```rust
    let unmanaged: Vec<&str> = all_files_on_disk
        .iter()
        .filter(|f| !tracked_paths.contains(f.as_str()))
        .map(|f| f.as_str())
        .collect();

    let total = unmanaged.len();
    let dirs = group_untracked_by_mod_dir(&unmanaged);

    Ok((dirs, total))
```

Then update `check_integrity_from` in `src/health.rs` — replace lines 181-205 (the inline scan+group block) with:

```rust
    let all_disk_files = scan_mod_directories(spt_dir)?;
    let tracked_paths: std::collections::HashSet<&str> =
        tracked_files.iter().map(|f| f.file_path.as_str()).collect();

    let untracked: Vec<&str> = all_disk_files
        .iter()
        .filter(|f| !tracked_paths.contains(f.as_str()))
        .map(|f| f.as_str())
        .collect();

    let dir_counts = crate::cli::common::group_untracked_by_mod_dir(&untracked);

    let untracked_dirs: Vec<UntrackedDir> = dir_counts
        .into_iter()
        .map(|(path, file_count)| UntrackedDir { path, file_count })
        .collect();
```

This preserves the existing function signature (`tracked_files: &[InstalledFile], spt_dir: &Path`) while eliminating the duplicated grouping logic.

- [ ] **Step 3: Run full test suite + clippy**

Run: `cargo test && cargo clippy`
Expected: PASS — `check_integrity_detects_untracked_files` test in health.rs should still pass with the correct `SPT/user/mods/UnknownMod` grouping.

- [ ] **Step 4: Commit**

```bash
git add src/cli/common.rs src/health.rs
git commit -m "refactor: unify unmanaged-dir grouping logic between health and common"
```

---

### Task 6: Add missing DB index and fix N+1 query

**Files:**
- Create: `migrations/003_file_mod_id_index.sql`
- Modify: `src/db/schema.rs` (add migration 003)
- Modify: `src/db/mods.rs` (add `list_mods_with_file_counts`)
- Modify: `src/web/handlers/mods.rs` (use new query in `list_mods` handler)

**Interfaces:**
- Produces: `Database::list_mods_with_file_counts() -> rusqlite::Result<Vec<(InstalledMod, usize)>>`

- [ ] **Step 1: Create migration**

Create `migrations/003_file_mod_id_index.sql`:

```sql
CREATE INDEX IF NOT EXISTS idx_installed_files_mod_id ON installed_files(mod_id);
```

- [ ] **Step 2: Register migration in schema.rs**

Add to `src/db/schema.rs`:

```rust
const MIGRATION_003: &str = include_str!("../../migrations/003_file_mod_id_index.sql");

// In run_migrations, after the existing if blocks:
    if current_version < 3 {
        conn.execute_batch(MIGRATION_003)?;
        conn.pragma_update(None, "user_version", 3)?;
    }
```

- [ ] **Step 3: Add `list_mods_with_file_counts` to `src/db/mods.rs`**

```rust
    pub fn list_mods_with_file_counts(&self) -> rusqlite::Result<Vec<(InstalledMod, usize)>> {
        let mut stmt = self.conn.prepare(
            "SELECT m.id, m.forge_mod_id, m.forge_version_id, m.name, m.slug, m.version,
                    m.installed_at, m.updated_at, COUNT(f.id) as file_count
             FROM installed_mods m
             LEFT JOIN installed_files f ON f.mod_id = m.id
             GROUP BY m.id
             ORDER BY m.name",
        )?;
        let rows = stmt.query_map([], |row| {
            let m = row_to_installed_mod(row)?;
            let count: usize = row.get(8)?;
            Ok((m, count))
        })?;
        rows.collect()
    }
```

- [ ] **Step 4: Update `list_mods` web handler to use new query**

In `src/web/handlers/mods.rs`, replace the `list_mods` handler's `web::block` closure:

```rust
    let mods = web::block(move || {
        let db = db.lock();
        let mods_with_counts = db.list_mods_with_file_counts()?;
        let entries = mods_with_counts
            .into_iter()
            .map(|(mod_info, file_count)| ModListEntry {
                mod_info,
                file_count,
            })
            .collect();
        Ok::<_, anyhow::Error>(entries)
    })
    .await
    .map_err(WebError::from)?
    .map_err(WebError::from)?;
```

- [ ] **Step 5: Run full test suite + clippy**

Run: `cargo test && cargo clippy`
Expected: PASS

- [ ] **Step 6: Commit**

```bash
git add migrations/003_file_mod_id_index.sql src/db/schema.rs src/db/mods.rs src/web/handlers/mods.rs
git commit -m "perf: add index on installed_files.mod_id, replace N+1 query with JOIN"
```

---

### Task 7: Fix `register_submit` use_invite bug

The current `register_submit` calls `use_invite(&code, 0)` to atomically consume the invite before the user exists, then tries `use_invite(&code, user_id)` to update with the real ID — but the second call always matches 0 rows (WHERE used_by IS NULL, but used_by is 0). Every invite ends up with `used_by=0`.

**Files:**
- Modify: `src/db/users.rs` (add `update_invite_user` method)
- Modify: `src/web/handlers/auth.rs` (fix `register_submit`, lines 280-302)

**Interfaces:**
- Produces: `Database::update_invite_user(code: &str, user_id: i64) -> rusqlite::Result<usize>` (unconditional UPDATE, no IS NULL guard)

- [ ] **Step 1: Add `update_invite_user` to `src/db/users.rs`**

```rust
    pub fn update_invite_user(&self, code: &str, user_id: i64) -> rusqlite::Result<usize> {
        self.conn.execute(
            "UPDATE invite_codes SET used_by = ?1 WHERE code = ?2",
            params![user_id, code],
        )
    }
```

- [ ] **Step 2: Fix the registration flow in `src/web/handlers/auth.rs`**

Replace lines 289-299 in the `web::block` closure:

```rust
        // Consume the invite atomically (prevents race with concurrent registrations)
        let used = db.use_invite(&code, 0)?;
        if used == 0 {
            return Ok(Err("Invite code is invalid or expired".to_string()));
        }

        let user_id = db.insert_user(&username, &profile_id, Some(&password_hash), "player")?;

        // Update the invite to point to the real user_id (no IS NULL guard needed)
        db.update_invite_user(&code, user_id)?;
```

- [ ] **Step 3: Run full test suite + clippy**

Run: `cargo test && cargo clippy`
Expected: PASS

- [ ] **Step 4: Commit**

```bash
git add src/db/users.rs src/web/handlers/auth.rs
git commit -m "fix: register_submit now correctly records user_id on invite after creation"
```

---

## Review findings addressed by this plan

| # | Finding | Task |
|---|---------|------|
| 1 | Web update deletes before extracting (no staging) | Task 3 |
| 2 | Web queue drain skips reverse-dep cleanup on remove | Task 4 |
| 3 | Unknown queue actions silently consumed | Task 4, Step 5 |
| 4 | N+1 queries + missing index on installed_files.mod_id | Task 6 |
| 5 | use_invite user_id=0 permanently wrong | Task 7 |
| 6 | Unmanaged-dir grouping logic diverges between health.rs and common.rs | Task 5 |

## Review findings NOT addressed (separate work)

| Finding | Why separate |
|---------|-------------|
| Web queue install skips dependency resolution | Requires async refactor — `install_with_deps` makes Forge API calls that can't run inside `web::block`. Deferred. |
| Session fixation (no session.renew on login) | Security fix, not DRY |
| JSONC comment stripping corrupts URLs | Requires a proper JSONC parser — scope creep |
| Path traversal validate_dest_under_root fails open | Security hardening, not DRY |
| Missing admin auth on /api partials | Security fix, not DRY |
| No CSRF protection | Security fix, not DRY |
