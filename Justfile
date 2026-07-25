default:
    @just --list

build:
    cargo build

check:
    cargo check

test:
    cargo test

clippy:
    cargo clippy -- -D warnings

fmt:
    cargo fmt

check-logging:
    ./scripts/check-logging.sh

cpd:
    jscpd .

lint: fmt clippy check-logging cpd

# Set up git hooks for local CI linting
install-hooks:
    git config core.hooksPath .githooks
    @echo "Git hooks installed from .githooks/"

run *ARGS:
    cargo run -- {{ARGS}}

serve *ARGS:
    cargo run -- serve {{ARGS}}

audit:
    cargo audit

release-dry-run:
    dist build

# Generate CHANGELOG.md from commit history
changelog:
    git-cliff --output CHANGELOG.md

# Preview changelog for next release (unreleased changes only)
changelog-preview:
    git-cliff --unreleased

# --- Development ---

dev_dir := ".dev-server"

# Worktree-aware defaults for parallel dev environments.
# Main repo → port 9190, container "spt-server-dev"
# Worktree  → deterministic port 9191-9289, container "spt-server-<worktree-name>"
_wt_name := `cd=$(git rev-parse --git-common-dir 2>/dev/null); gd=$(git rev-parse --git-dir 2>/dev/null); if [ "$cd" != "$gd" ]; then basename "$(git rev-parse --show-toplevel)"; fi`
dev_port := env("QUMA_DEV_PORT", `cd=$(git rev-parse --git-common-dir 2>/dev/null); gd=$(git rev-parse --git-dir 2>/dev/null); if [ "$cd" != "$gd" ]; then n=$(basename "$(git rev-parse --show-toplevel)"); echo $((9191 + $(printf '%s' "$n" | cksum | cut -d' ' -f1) % 99)); else echo 9190; fi`)
dev_container := env("QUMA_DEV_CONTAINER", `cd=$(git rev-parse --git-common-dir 2>/dev/null); gd=$(git rev-parse --git-dir 2>/dev/null); if [ "$cd" != "$gd" ]; then echo "spt-server-$(basename "$(git rev-parse --show-toplevel)")"; else echo spt-server-dev; fi`)
headless_dir := env("QUMA_HEADLESS_DIR", `echo "$HOME/Games/SPTarkov"`)

# Verify dev prerequisites are installed
dev-check:
    #!/usr/bin/env bash
    set -euo pipefail
    ok=true
    for cmd in podman fuse-overlayfs fusermount3 cargo sqlite3; do
        if command -v "$cmd" >/dev/null 2>&1; then
            echo "  $cmd: $(command -v $cmd)"
        else
            echo "  $cmd: MISSING"
            ok=false
        fi
    done
    if $ok; then
        echo "All prerequisites met."
    else
        echo ""
        echo "Install missing tools before running dev recipes."
        echo "  Fedora: sudo dnf install fuse-overlayfs fuse3 podman sqlite"
        exit 1
    fi

# Show dev environment settings for this worktree
dev-info:
    #!/usr/bin/env bash
    echo "dev_dir:       {{dev_dir}}"
    echo "dev_port:      {{dev_port}}"
    echo "dev_container: {{dev_container}}"
    echo "worktree:      {{_wt_name}}{{if _wt_name == "" { " (main repo)" } else { "" } }}"
    # Show overlay mount status
    runtime_mount="{{dev_dir}}/runtimes/spt-server"
    if mountpoint -q "$runtime_mount" 2>/dev/null; then
        echo "spt_overlay:   mounted"
    else
        echo "spt_overlay:   not mounted"
    fi
    # Show headless config and container status
    config="{{dev_dir}}/quartermaster.toml"
    if grep -q '\[headless\]' "$config" 2>/dev/null; then
        count=$(podman ps --filter label=quma.managed-by=quartermaster-clients --format '{{{{.Names}}}}' 2>/dev/null | wc -l)
        echo "headless:      configured ($count running)"
    else
        echo "headless:      not configured"
    fi

# Bootstrap a real SPT dev environment via `quma setup`
dev-init:
    #!/usr/bin/env bash
    set -euo pipefail
    if [ -f "{{dev_dir}}/quartermaster.toml" ]; then
        echo "Dev environment already exists at {{dev_dir}}/"
        echo "Run 'just dev-reset-db' to wipe the database, or 'just dev-clean' to start over."
        exit 0
    fi
    cargo run -- setup --quma-dir "{{dev_dir}}" --admin-password devdevdev --dev \
        --container-name "{{dev_container}}" --spt-version latest

# Build and run the web server against the dev directory
dev-serve *ARGS: dev-init
    @echo "Dev server on port {{dev_port}} (container: {{dev_container}})"
    QUMA_DIR="{{dev_dir}}" QUMA_WEB_PORT="{{dev_port}}" cargo run -- serve {{ARGS}}

# Run any quma command against the dev directory
dev-cli *ARGS: dev-init
    QUMA_DIR="{{dev_dir}}" QUMA_WEB_PORT="{{dev_port}}" cargo run -- {{ARGS}}

# Install development tools (cargo-watch for auto-reload)
dev-install-tools:
    cargo install cargo-watch

# Auto-rebuild and restart the dev server on file changes
dev-watch *ARGS: dev-init
    @echo "Dev server on port {{dev_port}} (container: {{dev_container}})"
    QUMA_DIR="{{dev_dir}}" QUMA_WEB_PORT="{{dev_port}}" cargo watch -x 'run -- serve {{ARGS}}' -w src -w templates

# Seed the dev database with test data (wipes and repopulates)
dev-seed: dev-init
    #!/usr/bin/env bash
    set -euo pipefail
    command -v sqlite3 >/dev/null || { echo "Error: sqlite3 is required but not installed"; exit 1; }
    db="{{dev_dir}}/quartermaster.db"
    if [[ ! -f "$db" ]]; then
        echo "Error: dev database not found at $db"
        echo "Run 'just dev-serve' once to initialize the database, then try again."
        exit 1
    fi
    echo "Seeding dev database..."
    sqlite3 "$db" < dev/seed.sql
    echo "Database seeded."
    # Copy profile fixtures — use overlay upper dir so they're visible through fuse-overlayfs
    fixtures="dev/fixtures/profiles"
    target="{{dev_dir}}/overlays/runtime/upper/SPT/user/profiles"
    if [ -d "$fixtures" ] && [ "$(find "$fixtures" -name '*.json' 2>/dev/null | head -1)" ]; then
        mkdir -p "$target"
        cp "$fixtures"/*.json "$target/"
        count=$(find "$fixtures" -name '*.json' | wc -l)
        echo "Copied $count profile(s) to $target/"
    else
        echo "No profile fixtures found in $fixtures/ (add .json files there to seed profiles)"
    fi
    echo "Done."

# Wipe the dev database (keeps config and SPT structure)
dev-reset-db:
    rm -f "{{dev_dir}}/quartermaster.db" "{{dev_dir}}/quartermaster.db-journal" "{{dev_dir}}/quartermaster.db-wal"
    echo "Dev database wiped."

# Check SVM metadata coverage against upstream C# models
sync-svm-metadata svm_repo="$HOME/code/SVM":
    python3 scripts/sync-svm-metadata.py "{{svm_repo}}"

# Stop headless containers and remove overlay data
dev-headless-clean:
    #!/usr/bin/env bash
    set -euo pipefail
    # Stop and remove headless containers (labeled by quma)
    found=false
    for container in $(podman ps -a --filter label=quma.managed-by=quartermaster-clients --format '{{{{.Names}}}}' 2>/dev/null); do
        podman stop "$container" 2>/dev/null || true
        podman rm "$container" 2>/dev/null || true
        echo "Removed headless container: $container"
        found=true
    done
    if ! $found; then
        echo "No headless containers found."
    fi
    # Remove headless overlay data
    rm -rf "{{dev_dir}}/overlays/headless"
    echo "Headless environment cleaned."

# Configure headless client support against a local SPT client install
dev-headless-init headless_path=headless_dir: dev-init
    #!/usr/bin/env bash
    set -euo pipefail
    if ! [ -d "{{headless_path}}" ]; then
        echo "Error: Headless client directory not found at {{headless_path}}"
        echo "Set QUMA_HEADLESS_DIR or pass the path: just dev-headless-init /path/to/spt-client"
        exit 1
    fi
    echo "Configuring headless client with install dir: {{headless_path}}"

    # Append headless config to quartermaster.toml if not already present
    config="{{dev_dir}}/quartermaster.toml"
    if ! grep -q '\[headless\]' "$config" 2>/dev/null; then
        # Ensure trailing newline before appending
        [ -n "$(tail -c1 "$config")" ] && echo >> "$config"
        printf '\n[headless]\ninstall_dir = "%s"\n' "{{headless_path}}" >> "$config"
        echo "Added [headless] section to $config"
    else
        echo "[headless] section already exists in $config"
    fi

    echo ""
    echo "Headless configured. To spin up a client:"
    echo "  1. Start the SPT server:  just dev-cli server start"
    echo "  2. Scale up:              just dev-cli headless scale 1"
    echo "  3. Start the client:      just dev-cli headless start 1"

# Remove the dev directory and container entirely
dev-clean: dev-headless-clean
    #!/usr/bin/env bash
    set -euo pipefail
    container="{{dev_container}}"
    # Unmount fuse-overlayfs if mounted
    runtime_mount="{{dev_dir}}/runtimes/spt-server"
    if mountpoint -q "$runtime_mount" 2>/dev/null; then
        fusermount3 -u "$runtime_mount"
        echo "Unmounted SPT overlay at $runtime_mount"
    fi
    # Stop and remove the dev container if it exists
    if podman inspect "$container" &>/dev/null; then
        podman stop "$container" 2>/dev/null || true
        podman rm "$container"
        echo "Container '$container' removed."
    elif docker inspect "$container" &>/dev/null; then
        docker stop "$container" 2>/dev/null || true
        docker rm "$container"
        echo "Container '$container' removed."
    fi
    rm -rf "{{dev_dir}}"
    echo "Dev environment removed."

# Build the minimal headless container image
build-headless:
    podman build -t localhost/quma-headless:latest container/headless/

# Build the SPT server container image
build-spt-server:
    podman build -t localhost/quma-spt-server:latest container/spt-server/
