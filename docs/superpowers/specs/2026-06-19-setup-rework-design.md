# Setup Rework Design

## Summary

Replace `quma setup` (12-step interactive wizard) and `quma init` with a single `quma setup` command that can bootstrap a brand new SPT/Fika server from nothing or wrap an existing SPT installation. The host answers at most three questions; everything else uses smart defaults.

## Motivation

The current setup assumes an SPT server already exists. A server host has to manually set up SPT, Fika, and containers before quma can manage anything. The new flow makes quma the single entry point — install quma, run `quma setup`, get a fully working server.

## User Interaction

```
$ quma setup

=== Quartermaster Setup ===

Where should server data live? [~/spt-server]: 
Install Fika for multiplayer? [Y/n]: 
Admin password (min 8 chars): ********
Confirm password: ********
```

- **Data directory**: Where SPT server files live. Default `~/spt-server`. Used as the container volume mount.
- **Fika prompt**: Skippable with `--no-fika` flag (skips the prompt entirely). Default yes.
- **Admin password**: Min 8 characters, entered via `rpassword` (no echo). Prompted twice for confirmation. Admin username defaults to `admin`.

The admin user is a web-only account for server management — no SPT profile link required. A profile can optionally be linked later.

## CLI Signature

```
quma setup [PATH] [--no-fika]
```

- `PATH` (optional positional): Data directory path. If provided, the data dir prompt is skipped. If omitted, the user is prompted with `~/spt-server` as the default.
- `--no-fika`: Skip Fika installation entirely (no prompt shown).

## Detection & Branching

After collecting input, quma checks the target directory state:

1. **Directory doesn't exist or is empty** → Path A (bootstrap from scratch).
2. **Directory exists and passes `validate_spt_dir`** → Path B (wrap existing).
3. **Directory exists, non-empty, but not a valid SPT install** → Fail with error: "Directory exists and contains files but is not a valid SPT installation. Use an empty directory for a fresh setup, or point at an existing SPT install."

A container runtime (Podman or Docker) is required for both paths. If `ContainerManager::new()` fails, setup fails with an actionable error: "No container runtime found. Install Podman or Docker and ensure the socket is enabled."

### Path A — Bootstrap (empty or nonexistent directory)

The directory is empty or doesn't exist. Quma stands up everything from scratch.

**Steps:**

1. Create the data directory (if it doesn't exist).
2. Connect to container runtime, verify socket is available.
3. Check if a container named `spt-server` already exists. If it does, fail with error: "Container 'spt-server' already exists. Remove it with `podman rm spt-server` or `docker rm spt-server` and re-run setup."
4. Pull `ghcr.io/zhliau/fika-spt-server-docker:latest`.
5. Create container `spt-server` with:
   - Image: `ghcr.io/zhliau/fika-spt-server-docker:latest`
   - Volume: `<data_dir>:/opt/server:rw,Z` (SELinux private label)
   - Port: `6969:6969/tcp`
   - Env: `LISTEN_ALL_NETWORKS=true`, `FIKA_MODE=install` (or `disabled` if `--no-fika`)
   - Label: `managed-by=quma`
6. Start container for first boot. SPT initializes and Fika installs (if enabled) inside the container.
7. Wait for SPT server to become ready — poll `SptClient::ping()` every 3s, 180s timeout. Connection errors during polling are swallowed as "not ready yet"; only the timeout is fatal. Show progress.
8. Stop container after first boot completes.
9. Create `quartermaster.toml` config with `spt_dir`, `server_container=spt-server`, `session_secret`, `server_host=0.0.0.0`, `server_port=6969`.
10. Create `quartermaster.db`, run migrations.
11. Create admin user with username `admin`, the collected password, and `spt_profile_id = NULL`.
12. Print summary.

### Path B — Wrap Existing (valid SPT directory detected)

The directory contains a valid SPT installation. Quma wraps around it.

**Steps:**

1. Read SPT version info via `read_spt_version`.
2. Detect existing container via `detect_spt_containers` (by volume mount):
   - Exactly one found → use it.
   - None found → check for `spt-server` name collision (fail if taken), then create one (same defaults as Path A, pointing at existing data dir).
   - Multiple found → prefer the one with `managed-by=quma` label, fall back to first.
3. Create `quartermaster.toml` config (same fields as Path A, using detected/created container name and network info from `http.json` if present).
4. Create `quartermaster.db`, run migrations.
5. Scan for unmanaged mods (existing `find_unmanaged_mod_dirs`).
6. Create admin user with username `admin`, `spt_profile_id = NULL`.
7. Print summary.

## Container Details

| Setting | Value |
|---------|-------|
| Name | `spt-server` |
| Image | `ghcr.io/zhliau/fika-spt-server-docker:latest` |
| Volume | `<data_dir>:/opt/server:rw,Z` |
| Port | `6969:6969/tcp` |
| Env | `LISTEN_ALL_NETWORKS=true`, `FIKA_MODE=install` or `disabled` |
| Label | `managed-by=quma` |

The `latest` tag is used for now. Pinning to a specific SPT version tag is a future enhancement.

Fika installation is handled by the container via `FIKA_MODE=install`. Fika integration in quma's web UI (tracking, version management) is deferred to future work.

## Schema Changes

### Migration 008: Make `spt_profile_id` nullable

SQLite doesn't support `ALTER COLUMN`, so this requires table recreation wrapped in a transaction:

```sql
BEGIN;

CREATE TABLE users_new (
    id              INTEGER PRIMARY KEY AUTOINCREMENT,
    username        TEXT NOT NULL UNIQUE,
    spt_profile_id  TEXT,
    password_hash   TEXT,
    role            TEXT NOT NULL DEFAULT 'player',
    disabled        INTEGER NOT NULL DEFAULT 0,
    created_at      TEXT NOT NULL DEFAULT (datetime('now')),
    password_changed_at TEXT
);

INSERT INTO users_new (id, username, spt_profile_id, password_hash, role, disabled, created_at, password_changed_at)
    SELECT id, username, spt_profile_id, password_hash, role, disabled, created_at, password_changed_at
    FROM users;

DROP TABLE users;
ALTER TABLE users_new RENAME TO users;

COMMIT;
```

### Code changes

- **`src/db/users.rs`**: `User` struct changes `spt_profile_id: String` to `spt_profile_id: Option<String>`. `insert_user` signature changes to accept `Option<&str>`. `row_to_user` updated to handle nullable column.
- **`src/web/handlers/admin.rs`**: Update all `spt_profile_id` accesses (e.g., `.is_empty()` → `.is_none()`).
- **`src/web/handlers/auth.rs`**: Registration handler passes `Some(&profile_id)` instead of `&profile_id`.
- **Templates**: Any template displaying `spt_profile_id` handles the `None` case.

## What Gets Removed

### Commands removed
- `quma init` — fully replaced by `quma setup`.

### Current `quma setup` code removed
- `configure_container()` — no interactive container selection; quma creates or auto-detects.
- `configure_networking()` — container handles this via `LISTEN_ALL_NETWORKS=true`.
- `configure_fika()` — host edits `fika.jsonc` post-setup if needed.
- `first_boot()` confirmation prompt — first boot is automatic.
- `detect_spt_directory()` interactive prompt — replaced by the data dir question upfront.
- `install_fika()` — container handles Fika installation via `FIKA_MODE`.

### Removed flags
- `--non-interactive`: No longer needed; the flow is inherently minimal.
- `--skip-fika`: Replaced by `--no-fika`.

## Files Changed

| File | Change |
|------|--------|
| `src/cli/setup.rs` | Rewritten with new flow |
| `src/cli/init.rs` | Deleted |
| `src/cli/mod.rs` | Remove `Init` command, update `Setup` flags |
| `src/db/users.rs` | `User.spt_profile_id` → `Option<String>`, `insert_user` accepts `Option<&str>`, `row_to_user` updated |
| `src/web/handlers/admin.rs` | Update `spt_profile_id` accesses for `Option` |
| `src/web/handlers/auth.rs` | Registration passes `Some(&profile_id)` |
| `src/cli/serve.rs` | Error message: `quma setup` instead of `quma init` |
| `migrations/008_nullable_profile_id.sql` | Make `spt_profile_id` nullable |
| Templates referencing `spt_profile_id` | Handle `None` case |

## Files Unchanged

- `src/container.rs` — `ContainerManager` used as-is (has `pull_image`, `create_container`, etc.)
- `src/spt/detect.rs` — `validate_spt_dir`, `read_spt_version`, `detect_spt_dir` used as-is
- `src/forge/` — Not used during setup (Fika handled by container)
- `src/spt/profiles.rs` — Not used during setup (admin is profile-less)

## Summary Output

After setup completes:

```
=== Setup Complete ===

SPT directory: ~/spt-server
Container: spt-server
Fika: installed
Web UI: http://0.0.0.0:9190
Admin user: admin

Next steps:
  quma serve              Start the web UI
  quma server start       Start the SPT server
  quma invite             Generate invite codes for players

Network requirements (for multiplayer):
  TCP 6969 inbound        SPT server
  UDP 25565 inbound       Fika P2P raids (whoever hosts)
  Consider UPnP or VPN as alternatives to port forwarding.
```

## Out of Scope

- Web-based setup wizard (future work).
- Pinning container image to specific SPT version tag.
- Fika tracking in quma's database / web UI integration (future work).
- Fika config editing during setup (`fika.jsonc` settings).
- "Link profile" mechanism for admin users (future web UI feature).
- Player onboarding / invite flow improvements.
