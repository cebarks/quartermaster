# Container Creation Support

## Summary

Add the ability to create a properly configured SPT server container via `quma setup` and `quma server create`. Currently quartermaster can only detect and select existing containers — users must manually run `podman create` with the correct flags, which is error-prone (as demonstrated by `--user root`, env var, and permission issues).

## Scope

- `PodmanClient` gains `pull_image()` and `create_spt_container()` methods
- `quma setup` offers container creation when no container is detected
- `quma server create` as a standalone command for (re)creating the container
- **Out of scope**: Web UI container creation (deferred to a future setup wizard), Fika-specific env vars (managed through web UI)

## Design

### 1. PodmanClient (`src/podman.rs`)

Two new associated functions (no `&self` — the container doesn't exist yet):

**`pull_image(image: &str) -> Result<()>`**
- Runs `podman pull <image>`
- Always pulls (no local cache check)
- Returns error if pull fails

**`create_spt_container(name: &str, spt_dir: &Path, port: u16) -> Result<()>`**
- Runs `podman create` with:
  - `--name <name>`
  - `-p <port>:6969`
  - `-v <spt_dir>:/opt/server`
  - `--user root`
  - `-e TAKE_OWNERSHIP=true`
  - `-e CHANGE_PERMISSIONS=true`
  - `-e LISTEN_ALL_NETWORKS=true`
  - Image: `ghcr.io/zhliau/fika-spt-server-docker:latest`
- Returns error if creation fails (e.g., name already taken)

Constants:
- `const SPT_SERVER_IMAGE: &str = "ghcr.io/zhliau/fika-spt-server-docker:latest";`
- `const DEFAULT_CONTAINER_NAME: &str = "spt-server";`
- `const DEFAULT_SPT_PORT: u16 = 6969;`

### 2. Setup CLI (`src/cli/setup.rs`)

Modify `configure_container()` to add a creation branch. The flow becomes:

1. If `server_container` already set in config → keep it, return
2. Detect existing containers via `detect_spt_containers()`
3. If exactly one found → offer to use it
4. If multiple found → let user pick
5. If none found (or user declined) → **offer to create one**:
   - Prompt for container name (default: `"spt-server"`)
   - Prompt for host port (default: `6969`)
   - Pull the image
   - Create the container
   - Set `config.server_container = Some(name)`
6. If user declines creation → allow manual name entry (existing behavior)

In non-interactive mode: auto-create with defaults when no container is detected.

### 3. Standalone CLI Command (`src/cli/server.rs`)

Add `quma server create` subcommand:

```
quma server create [--name <name>] [--port <port>]
```

- `--name`: Container name (default: `"spt-server"`)
- `--port`: Host port to map (default: `6969`)
- Uses `spt_dir` from CLI context
- Pulls the image, creates the container
- Updates `server_container` in config if not already set, saves config
- Errors if a container with that name already exists (user should `podman rm` first)

### 4. Error Handling

- **Image pull failure**: Report network/registry error, suggest retry
- **Container name conflict**: Report that the name is taken, suggest `podman rm <name>` or `--name <other>`
- **Podman not installed**: Existing pattern — command fails with "failed to run podman ..."
- **Permission issues**: Unlikely with `--user root`, but surface podman's stderr in error message

### 5. Testing

- Unit tests for `pull_image` and `create_spt_container` argument construction (mock the command execution or test argument building separately)
- Integration test for `configure_container` creation flow in non-interactive mode
- Existing `configure_container` tests remain unchanged
