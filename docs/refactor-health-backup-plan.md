# Refactoring Plan: health.rs + backup.rs Duplication Reduction

## Baseline
- 32 clones, 2,388 duplicated tokens (16.09% of 14,841 total)
- health.rs: ~1,217 lines, backup.rs: ~1,103 lines

---

## Cluster 1: backup.rs — File copy loops in `backup_mod` (lines 117-146)

**What's duplicated**: Two nearly identical loops copying files from `spt_dir` to backup destination. The mod-file loop (117-130) and addon-file loop (133-146) share identical logic: join paths, create parent dirs, check exists, copy, accumulate size, warn on missing.

**Fix**: Extract a `copy_files_to_backup()` helper that takes `&[InstalledFile]`, `spt_dir`, `dest`, and returns the total size copied. Call it twice — once for mod files, once for addon files.

**Risk**: Low. Pure file-copy logic with no branching differences.

---

## Cluster 2: backup.rs — File copy loops in `backup_full` (lines 210-221, 242-253)

**What's duplicated**: Two loops copying mod files and addon files during full backup. Both iterate files, join paths, create parent dirs, copy if exists, accumulate size, and collect `file_paths`. Nearly identical structure.

**Fix**: Extract a `copy_files_for_manifest()` helper that takes `&[InstalledFile]`, `spt_dir`, `dest/mods`, and returns `(i64, Vec<String>)` (total_size, file_paths). Call it for both mod and addon file lists.

**Risk**: Low. Same pattern, just different file sources.

---

## Cluster 3: backup.rs — Manifest file restore loops in `restore_full_backup` (lines 530-546, 583-598)

**What's duplicated**: Two loops restoring files from backup manifest — one for mods (insert_file), one for addons (insert_addon_file). Both: check src exists, warn if missing, create parent dirs, copy, read content, compute hash, compute size, insert DB record.

**Fix**: Extract a `restore_manifest_files()` helper that takes the file paths, source dir, spt_dir, and a closure `Fn(rel_path, hash, size) -> Result<()>` for the DB insert. Each caller passes the appropriate DB insert call as the closure.

**Risk**: Low-medium. The closure approach is idiomatic Rust and avoids trait complexity.

---

## Cluster 4: backup.rs — Retention functions (lines 710-755)

**What's duplicated**: `enforce_retention_mod` and `enforce_retention_full` share identical structure: check max_backups == 0, while loop counting vs limit, get oldest, remove dir, delete DB record.

**Fix**: Extract a generic `enforce_retention()` that takes closures for `count` and `oldest` queries. Both callers pass their specific DB query closures.

**Risk**: Low. The functions are structurally identical with only the DB query calls differing.

---

## Cluster 5: health.rs — ServerHealth construction (lines 121-146)

**What's duplicated**: Two `ServerHealth` struct literals that share 6 of 8 fields identically (or with only None vs computed value differences). The "unreachable" early return and the "reachable" return both construct the full struct.

**Fix**: Implement `Default` for `ServerHealth` (with `address: String::new()`) and use struct update syntax. Build a base struct, then override the fields that differ. Alternatively, construct the common fields once and use conditional assignment for `version`, `version_matches`, and `error`.

**Risk**: Low. The two code paths are already clearly related.

---

## Cluster 6: health.rs tests — InstalledMod boilerplate (many tests)

**What's duplicated**: 15+ `InstalledMod { ... }` constructions with identical boilerplate fields (`slug: None`, `version: "1.0.0"`, `installed_at: "2026-01-01T00:00:00Z"`, `updated_at: None`, `disabled: false`). Only `id`, `forge_mod_id`, `forge_version_id`, `name`, and occasionally `disabled` vary.

**Fix**: Add a `test_mod(id, forge_mod_id, name)` helper function in the test module. It sets sensible defaults for all other fields. Tests that need `disabled: true` can use struct update syntax: `InstalledMod { disabled: true, ..test_mod(2, 101, "X") }`.

**Risk**: None. Test-only change. Makes tests more readable.

---

## Cluster 7: health.rs tests — CliContext + integrity test setup (lines 706-841)

**What's duplicated**: Three integrity tests (`check_integrity_detects_missing_file`, `check_integrity_detects_modified_file`, `check_integrity_detects_untracked_files`) all create a temp dir, build identical directory structures, create a `CliContext` with the same fields, and call `check_integrity_from`. The CliContext construction is 12 lines of boilerplate each time.

**Fix**: Extract a `test_integrity_ctx(spt_dir)` or `test_cli_context(spt_dir)` helper that builds the CliContext. Each test still sets up its own temp dir and files (those differ per test), but the context construction is shared.

**Risk**: None. Test-only change.

---

## Cluster 8: backup.rs tests — Repeated test setup patterns (many tests)

**What's duplicated**: Multiple tests create the same setup: `tempfile::TempDir`, `create_dir_all("SPT/user/mods/TestMod")`, `write("package.json", b"{}")`, `Database::open_in_memory()`, `insert_mod(100, 200, "TestMod", ...)`, `insert_file(mod_id, "SPT/user/mods/TestMod/package.json", ...)`, `Config::default()`. This pattern appears 5+ times.

**Fix**: Extract a `TestBackupEnv` struct with a `new()` method that does all the common setup and returns the temp dir, db, mod_id, and config. Tests that need additional setup (profiles, config file, multiple mods) extend from there.

**Risk**: None. Test-only change.

---

## Implementation Order

1. Clusters 6, 7, 8 (test helpers) — zero risk, immediate payoff, good warm-up
2. Clusters 1, 2 (backup file copy helpers) — straightforward extraction
3. Cluster 4 (retention unification) — straightforward closure pattern
4. Cluster 5 (ServerHealth dedup) — small but clean
5. Cluster 3 (manifest restore) — slightly more complex closure pattern, do last

## Verification

After each cluster:
- `cargo check` — compilation
- `cargo test` — all tests pass
- `cargo clippy -- -D warnings` — no new warnings

After all clusters:
- `jscpd src/health.rs src/backup.rs --reporters console` — measure improvement
