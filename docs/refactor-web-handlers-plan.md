# Web Handlers Refactoring Plan

## Baseline Metrics
- **107 clones**, **8,855 duplicated tokens (20.96%)**, **1,214 duplicated lines (18.08%)**
- 6 files analyzed: `mods.rs`, `clients.rs`, `admin.rs`, `svm.rs`, `requests.rs`, `settings.rs`

## Refactoring Groups

### Group 1: clients.rs — Container Manager + Client Resolution (est. -800 tokens)

**What's duplicated**: Three handlers (`client_restart`, `client_stop`, `client_start`) have identical blocks:
1. Check `state.container_mgr.as_ref()` with flash error + redirect (~10 lines each)
2. Look up `client_states` to find container name by index (~10 lines each)
3. Handle "not found" with flash + redirect (~8 lines each)

Also: `client_create`, `client_delete`, `client_scale` repeat the container_mgr check.

**Why**: Copy-paste when adding start/stop/restart handlers.

**Refactoring**:
- Extract `require_container_mgr(state, session) -> Result<&ContainerManager, HttpResponse>` (matches server.rs pattern)
- Extract `resolve_client_container(state, index) -> Option<String>` to get container name from client states
- Combine into a `require_client_container(state, session, index) -> Result<(String, &ContainerManager), HttpResponse>` that does both in one call
- Each handler shrinks from ~50 lines to ~20 lines

**Risk**: Low. Pure extraction, no behavior change.

### Group 2: mods.rs — ModListEntry Construction (est. -200 tokens)

**What's duplicated**: `ModListEntry` construction from `(InstalledMod, usize, i64)` tuples with addon_counts lookup appears 3 times (list_mods line 288, line 307, list_body_partial line 1760).

**Refactoring**:
- Add a `ModListEntry::from_tuple(tuple: (InstalledMod, usize, i64), addon_counts: &HashMap<i64, usize>) -> Self` constructor.

**Risk**: Very low.

### Group 3: mods.rs — Empty Carousel Template (est. -200 tokens)

**What's duplicated**: Empty `UpdatesCarouselTemplate { user, entry: None, total: 0, ... }` constructed 3 times in `updates_carousel_partial` (lines 605, 634, 670).

**Refactoring**:
- Extract `fn empty_carousel(user: SessionUser, csrf_token: String) -> UpdatesCarouselTemplate` helper.

**Risk**: Very low.

### Group 4: mods.rs — Queue Operation Pattern (est. -400 tokens)

**What's duplicated**: In `install_mod`, `update_mod`, `remove_mod`, `install_addon`, `update_addon`: the pattern of checking `should_queue` + calling `insert_pending_op` + handling `already_queued` + flash message + redirect. ~25 lines repeated 5 times with minor variations (action type, redirect URL, flash message).

**Refactoring**:
- Extract `try_queue_mod_op(state, session, db, action, forge_mod_id, version_id, mod_name, redirect) -> Option<HttpResponse>` that returns Some(redirect) if queued, None if should proceed with immediate operation.
- Similar `try_queue_addon_op` for addon variants.

**Risk**: Medium. Need to be careful with the different `QueueAction` variants and redirect URLs. Each call site has slightly different parameters.

### Group 5: settings.rs — Config Save Pattern (est. -350 tokens)

**What's duplicated**: All 6 save handlers (`save_web_settings`, `save_server_settings`, `save_queue_settings`, `save_forge_settings`, `save_logging_settings`, `save_headless_settings`) follow the same pattern:
1. Auth + permission + CSRF check (identical 5 lines)
2. `let _guard = state.config_lock.lock()`
3. `Config::load(&state.config_path).map_err(WebError::from)?`
4. Modify config fields
5. `config.save(&state.config_path).map_err(WebError::from)?`
6. `state.update_config_from_disk()` with warning log
7. Flash message + redirect

**Refactoring**:
- Extract `save_config(state, config) -> Result<(), WebError>` that handles steps 5-6.
- The auth/CSRF check is 5 lines and while repetitive, extracting it into a helper would require passing session/req/form references which is messy with actix types. Leave the auth/CSRF check as-is.

**Risk**: Low. The config mutation is handler-specific so that stays. Only the save+refresh boilerplate is extracted.

### Group 6: admin.rs — Users + Profiles Computation (est. -300 tokens)

**What's duplicated**: `admin_page` and `admin_users` both:
1. Query DB for users + roles
2. Load profile stats
3. Call `build_user_profiles` + `compute_available_profiles`
4. Zip users with profiles

~20 lines duplicated.

**Refactoring**:
- Extract `load_users_with_profiles(state) -> Result<(Vec<(User, ProfileStatus)>, Vec<RoleRecord>, Vec<SptProfile>), WebError>` that does all four steps.

**Risk**: Low. Pure extraction.

### Group 7: Cross-file mods.rs <-> requests.rs — Install Logic (est. -500 tokens)

**What's duplicated**: `trigger_install_for_request` in requests.rs is a near-duplicate of the install task spawning in `mods::install_mod`. The core pattern (~40 lines) is identical: resolve deps, download, extract, insert into DB, record dep edges, scan runtime files, regenerate modsync, update caches, handle SVM.

**Refactoring**:
- This is explicitly marked as `TODO(debt)` in both files. However, extracting this requires moving the spawn logic to `ops.rs` or a shared module, which is outside our target file list.
- **Defer this** — it would require modifying `ops.rs` which is outside scope. Leave the TODO.

**Risk**: N/A (deferred).

### Group 8: svm.rs — SVM Lock Accessor (est. -100 tokens)

**What's duplicated**: `state.svm.as_ref().ok_or(WebError::NotFound)?` appears 7+ times.

**Refactoring**:
- Add a method `AppState::require_svm(&self) -> Result<&parking_lot::RwLock<SvmManager>, WebError>` on AppState. But this modifies state.rs which is outside target.
- Alternative: local helper `fn require_svm(state: &AppState) -> actix_web::Result<&parking_lot::RwLock<SvmManager>>` in svm.rs.

**Risk**: Very low.

## Implementation Order

1. Group 1 (clients.rs) — highest impact, cleanest extraction
2. Group 5 (settings.rs) — clean pattern, easy to verify
3. Group 6 (admin.rs) — straightforward extraction
4. Group 2 + 3 (mods.rs constructors) — simple
5. Group 4 (mods.rs queue pattern) — medium complexity
6. Group 8 (svm.rs) — small but clean

## Files to Modify
- `src/web/handlers/clients.rs` — Groups 1
- `src/web/handlers/settings.rs` — Group 5
- `src/web/handlers/admin.rs` — Group 6
- `src/web/handlers/mods.rs` — Groups 2, 3, 4
- `src/web/handlers/svm.rs` — Group 8

## Constraints
- No route signature changes
- No public API behavior changes
- All tests must pass
- `cargo check`, `cargo test`, `cargo clippy -- -D warnings` must all pass
- Don't modify files outside target list (except handlers/mod.rs if needed for re-exports)
