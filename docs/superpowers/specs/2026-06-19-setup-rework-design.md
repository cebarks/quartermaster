# Setup Rework Design

## Summary

Replace `quma setup` (12-step interactive wizard) and `quma init` with a single `quma setup` command that can bootstrap a brand new SPT/Fika server from nothing or wrap an existing SPT installation. The host answers at most three questions; everything else uses smart defaults.

## Motivation

The current setup assumes an SPT server already exists. A server host has to manually set up SPT, Fika, and Podman containers before quma can manage anything. The new flow makes quma the single entry point — install quma, run `quma setup`, get a fully working server.

## User Interaction

```
$ quma setup

=== Quartermaster Setup ===

Where should server data live? [~/spt-server]: 
Install Fika for multiplayer? [Y/n]: 
Admin password (min 8 chars): ********
```

- **Data directory**: Where SPT server files live. Default `~/spt-server`. Used as the container volume mount.
- **Fika prompt**: Skippable with `--no-fika` flag (skips the prompt entirely). Default yes.
- **Admin password**: Min 8 characters, entered via `rpassword` (no echo). Admin username defaults to `admin`.

## Detection & Branching

After collecting input, quma checks whether the data directory contains a valid SPT installation (using existing `spt::detect::validate_spt_dir`).

### Path A — Bootstrap (empty or nonexistent directory)

The directory is empty or doesn't exist. Quma stands up everything from scratch.

**Steps:**

1. Create the data directory (if it doesn't exist).
2. Connect to Podman, verify socket is available.
3. Pull `ghcr.io/zhliau/fika-spt-server-docker:latest`.
4. Create container `spt-server` with:
   - Image: `ghcr.io/zhliau/fika-spt-server-docker:latest`
   - Volume: `<data_dir>:/opt/server:rw,Z` (SELinux private label)
   - Port: `6969:6969/tcp`
   - Env: `LISTEN_ALL_NETWORKS=true`, `FIKA_MODE=disabled`
   - Label: `managed-by=quma`
5. Start container for first boot.
6. Wait for SPT server to become ready — poll `SptClient::ping()` every 3s, 180s timeout. Show progress.
7. Stop container after first boot completes.
8. Create `quartermaster.toml` config with `spt_dir`, `server_container`, `session_secret`, `server_host=0.0.0.0`, `server_port=6969`.
9. Create `quartermaster.db`, run migrations.
10. If Fika requested: install Fika via Forge (uses `ForgeClient` to find latest compatible version, installs with dependencies, tracked in DB).
11. Create admin user with username `admin`, the collected password, and `spt_profile_id = NULL`.
12. Print summary.

### Path B — Wrap Existing (valid SPT directory detected)

The directory contains a valid SPT installation. Quma wraps around it.

**Steps:**

1. Read SPT version info via `read_spt_version`.
2. Detect existing container via `detect_spt_containers` (by volume mount):
   - Exactly one found → use it.
   - None found → create one (same defaults as Path A, pointing at existing data dir).
   - Multiple found → prefer the one with `managed-by=quma` label, fall back to first.
3. Create `quartermaster.toml` config (same fields as Path A, using detected/created container name and network info from `http.json` if present).
4. Create `quartermaster.db`, run migrations.
5. Scan for unmanaged mods (existing `find_unmanaged_mod_dirs`).
6. If Fika requested and not already installed: install via Forge.
7. Create admin user:
   - No profiles exist → username `admin`, `spt_profile_id = NULL`.
   - One profile → auto-select it, use profile username.
   - Multiple profiles → prompt host to pick from numbered list.
8. Print summary.

## Container Details

| Setting | Value |
|---------|-------|
| Name | `spt-server` |
| Image | `ghcr.io/zhliau/fika-spt-server-docker:latest` |
| Volume | `<data_dir>:/opt/server:rw,Z` |
| Port | `6969:6969/tcp` |
| Env | `LISTEN_ALL_NETWORKS=true`, `FIKA_MODE=disabled` |
| Label | `managed-by=quma` |

The `latest` tag is used for now. Pinning to a specific SPT version tag is a future enhancement.

Fika installation is handled by quma (not the container) so it's tracked in the database like any other mod. The container runs with `FIKA_MODE=disabled`.

## Schema Changes

### Migration 008: Make `spt_profile_id` nullable

SQLite doesn't support `ALTER COLUMN`, so this requires table recreation:

```sql
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

INSERT INTO users_new SELECT * FROM users;
DROP TABLE users;
ALTER TABLE users_new RENAME TO users;
```

### Code changes in `db/users.rs`

- `insert_user` signature: `spt_profile_id: Option<&str>` instead of `&str`.
- All callers updated (setup, registration handler).

## CLI Changes

### `quma setup` new signature

```
quma setup [PATH] [--no-fika]
```

- `PATH` (optional positional): Data directory path. If provided, the data dir prompt is skipped. If omitted, the user is prompted with `~/spt-server` as the default.
- `--no-fika`: Skip Fika installation entirely (no prompt shown).

### Removed flags

- `--non-interactive`: No longer needed; the flow is inherently minimal.
- `--skip-fika`: Replaced by `--no-fika`.

### `quma init` removed

The `Init` command variant is deleted from `src/cli/mod.rs`. `quma init` no longer exists.

### `quma serve` error message update

Line in `src/cli/serve.rs` that says `Run 'quma init' first` updated to say `Run 'quma setup' first`.

## Files Changed

| File | Change |
|------|--------|
| `src/cli/setup.rs` | Rewritten with new flow |
| `src/cli/init.rs` | Deleted |
| `src/cli/mod.rs` | Remove `Init` command, update `Setup` flags |
| `src/db/users.rs` | `insert_user` accepts `Option<&str>` for `spt_profile_id` |
| `src/cli/serve.rs` | Error message: `quma setup` instead of `quma init` |
| `migrations/008_nullable_profile_id.sql` | Make `spt_profile_id` nullable |

## Files Unchanged

- `src/container.rs` — `ContainerManager` used as-is (has `pull_image`, `create_container`, etc.)
- `src/spt/detect.rs` — `validate_spt_dir`, `read_spt_version`, `detect_spt_dir` used as-is
- `src/forge/` — Forge client and mod install machinery used as-is
- `src/spt/profiles.rs` — `list_profiles` used as-is for existing-server path

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
- "Link profile" mechanism for admin users created without a profile (follow-up feature).
- Fika config editing during setup (`fika.jsonc` settings).
- Player onboarding / invite flow improvements.
