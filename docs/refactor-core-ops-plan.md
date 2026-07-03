# Refactoring Plan: Core Operations + SPT Duplication Reduction

## Baseline

- **60 clones**, **5,255 duplicated tokens** (16.63% of 31,598 total)
- Files: `src/ops.rs` (3,346 tokens), `src/spt/mods.rs` (982 tokens), `src/spt/profiles.rs` (765 tokens)
- Cross-file: `ops.rs` <-> `spt/mods.rs` (162 tokens)

---

## Refactoring 1: Extract `move_staged_files` helper (ops.rs)

**What**: The staging-to-live file copy loop appears 6 times in ops.rs:
- `install_mod_from_archive` (line 92-99)
- `install_addon_from_archive` (line 146-153)
- `update_mod_from_archive` (line 195-202)
- `update_addon_from_archive` (line 275-282)
- `apply_mod_update` step 2 (line 375-382)
- `apply_addon_update` step 2 (line 506-513)

**Pattern**:
```rust
for file in &extracted {
    let src = staging_dir.join(&file.path);
    let dst = spt_dir.join(&file.path);
    if let Some(parent) = dst.parent() {
        std::fs::create_dir_all(parent)?;
    }
    std::fs::rename(&src, &dst).or_else(|_| std::fs::copy(&src, &dst).map(|_| ()))?;
}
```

**How**: Extract to `fn move_staged_files(staging_dir: &Path, spt_dir: &Path, files: &[ExtractedFile]) -> Result<()>`. Replace all 6 occurrences.

**Risk**: Low. Pure filesystem operation, same behavior. All callers pass the same types.

**Estimated token reduction**: ~6 * 60 tokens saved, keeping 1 copy = ~300 tokens

---

## Refactoring 2: Extract `compute_and_remove_stale_files` helper (ops.rs)

**What**: The stale file computation + deletion logic appears 4 times:
- `update_mod_from_archive` (lines 205-214)
- `update_addon_from_archive` (lines 285-294)
- `apply_mod_update` step 2 (lines 384-393)
- `apply_addon_update` step 2 (lines 515-524)

**Pattern**:
```rust
let new_paths: HashSet<&str> = extracted.iter().map(|f| f.path.as_str()).collect();
let stale_paths: Vec<String> = old_paths.into_iter()
    .filter(|p| !new_paths.contains(p.as_str())).collect();
if !stale_paths.is_empty() {
    tracing::debug!(stale_count = stale_paths.len(), "removing stale files");
    crate::spt::mods::delete_mod_files(spt_dir, &stale_paths)?;
}
```

**How**: Extract to `fn remove_stale_files(spt_dir: &Path, old_paths: Vec<String>, new_files: &[ExtractedFile]) -> Result<()>`. Replace all 4 occurrences.

**Risk**: Low. Same logic, just wrapped in a function.

**Estimated token reduction**: ~4 * 80 - 80 = ~240 tokens

---

## Refactoring 3: Unify `record_extracted_files` / `record_extracted_addon_files` (ops.rs)

**What**: Two nearly identical functions (lines 10-36) that differ only in calling `db.insert_file` vs `db.insert_addon_file`.

**How**: Replace with a single function that takes a closure for the insert call:
```rust
fn record_files_with<F>(files: &[ExtractedFile], mut insert: F) -> Result<()>
where F: FnMut(&str, Option<&str>, Option<i64>) -> Result<()>
```

**Risk**: Low. The closure captures the DB and ID, so callers need minor adjustment but behavior is identical.

**Estimated token reduction**: ~80 tokens

---

## Refactoring 4: Extract `verify_new_files_on_disk` for recovery (ops.rs)

**What**: The file verification loop in recovery appears identically in both `recover_single_update` (lines 631-651) and `recover_single_addon_update` (lines 750-770).

**Pattern**: Iterates over new files, checks existence on disk, reads content, computes hash, counts matches.

**How**: Extract to `fn count_verified_files(spt_dir: &Path, new_files: &[ExtractedFile]) -> usize`. Returns the count of files that exist with correct hashes.

**Risk**: Low. Pure read-only operation.

**Estimated token reduction**: ~150 tokens

---

## Refactoring 5: Extract `cleanup_partial_copy` for recovery (ops.rs)

**What**: The partial copy cleanup logic appears in both recovery functions (lines 679-694 and lines 804-819).

**Pattern**: Iterates over new files, removes those not in old set.

**How**: Extract to `fn cleanup_partial_copy(spt_dir: &Path, new_files: &[ExtractedFile], old_paths: &[String])`.

**Risk**: Low. Same logic in both places.

**Estimated token reduction**: ~100 tokens

---

## Refactoring 6: Unify recovery decision logic (ops.rs)

**What**: After verifying files, both `recover_single_update` and `recover_single_addon_update` share identical decision branching (all_new_present -> complete forward, partial -> rollback, old_exist -> clear marker, ambiguous -> clear marker). The only difference is which DB calls are made in the "complete forward" branch and the tracing labels.

**How**: Extract the shared decision/action logic into a helper struct and function. The "complete forward" DB action is passed as a closure.

```rust
fn execute_recovery_decision(
    db: &Database,
    spt_dir: &Path,
    record: &PendingUpdate,
    new_files: &[ExtractedFile],
    old_paths: &[String],
    label: &str,           // "mod" or "addon"
    complete_db: impl FnOnce(&Database) -> Result<()>,
) -> Result<()>
```

This would let us collapse `recover_single_update` and `recover_single_addon_update` into thin wrappers that just do the entity-existence check and provide the DB closure.

**Risk**: Medium. Must ensure all tracing fields and DB calls are preserved exactly.

**Estimated token reduction**: ~400 tokens (the entire duplicated recovery bodies minus the unique parts)

---

## Refactoring 7: Extract stash tree-walking helper (profiles.rs)

**What**: `calculate_stash_value` (lines 303-339) and `load_stash_items` (lines 499-557) share the same tree-walking pattern:
- Build parent->children map
- Stack-based traversal from stash root
- Extract count via `upd.stack_objects_count.unwrap_or(1)`

**How**: Extract a helper that walks the stash tree and calls a visitor closure for each item:
```rust
fn walk_stash_items<F>(inventory: &InventoryData, mut visitor: F)
where F: FnMut(&InventoryItem, i64)  // item, count
```

Both callers become thin wrappers.

**Risk**: Low. Same traversal logic, just parameterized by what to do with each item.

**Estimated token reduction**: ~200 tokens

---

## Refactoring 8: Extract `create_test_zip` to shared test module (cross-file)

**What**: Identical `create_test_zip` helper in both `ops.rs::tests` (lines 1718-1731) and `spt/mods.rs::tests` (lines 691-704).

**How**: Make `create_test_zip` in `spt/mods.rs` public (`pub(crate)`) under `#[cfg(test)]` so `ops.rs::tests` can import it. Or create a `src/test_utils.rs` module. The former is simpler since `spt/mods.rs` is the natural home for archive-related test utilities.

**Risk**: Low. Test-only change, no production code affected.

**Estimated token reduction**: ~160 tokens

---

## Refactoring 9: Build rename lists for disable/enable (ops.rs)

**What**: The rename list construction for disable operations appears in `disable_mod` (lines 1217-1227), `disable_addon` (lines 1394-1404), and similarly in `enable_mod` (lines 1298-1318) and `enable_addon` (lines 1481-1501).

**How**: Extract two helpers:
- `fn build_disable_renames(spt_dir: &Path, top_dirs: &[String], loose: &[&str]) -> Vec<(PathBuf, PathBuf)>` — appends `.disabled`
- `fn build_enable_renames(spt_dir: &Path, top_dirs: &[String], loose: &[&str]) -> Vec<(PathBuf, PathBuf)>` — strips `.disabled`

**Risk**: Low. Pure data transformation, no side effects.

**Estimated token reduction**: ~200 tokens

---

## Execution Order

1. Refactoring 1 (move_staged_files) — most instances, highest impact
2. Refactoring 2 (remove_stale_files) — second most instances
3. Refactoring 3 (record_files unification)
4. Refactoring 8 (test helper dedup)
5. Refactoring 7 (stash tree walker)
6. Refactoring 4 (verify_new_files_on_disk)
7. Refactoring 5 (cleanup_partial_copy)
8. Refactoring 6 (unify recovery decision)
9. Refactoring 9 (build rename lists)

After each group, run `cargo check && cargo test && cargo clippy -- -D warnings`.

## Not Refactored (deliberate)

- **ZIP vs 7z extraction functions** (`extract_mod_zip` / `extract_mod_7z`): Structurally similar but differ in error handling (anyhow vs sevenz_rust2::Error), API shapes, and control flow. Unifying would require complex trait abstraction for diminishing returns.

- **Test setup helpers in profiles.rs** (`create_fake_profile`, `create_full_profile`, `create_detailed_profile`): Each creates a different JSON structure. Unifying would make tests less readable.

- **disable_mod/disable_addon and enable_mod/enable_addon full unification**: While they share structure, they differ in DB calls (mod vs addon APIs), backup strategies, modsync handling, and DB path update approaches (rename_file_path vs reprefix_addon_files). Full unification would require a trait or callback-heavy design that would hurt readability. The rename-list extraction (Refactoring 9) captures the cleanest shared parts.

- **install/update full function unification**: install and update have different DB transaction shapes and different pre/post steps (backup, stale file removal). Unifying would create a complicated dispatch that's harder to follow than the current explicit code.
