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

# Show dev environment settings for this worktree
dev-info:
    @echo "dev_dir:       {{dev_dir}}"
    @echo "dev_port:      {{dev_port}}"
    @echo "dev_container: {{dev_container}}"
    @echo "worktree:      {{_wt_name}}{{if _wt_name == "" { " (main repo)" } else { "" } }}"

# Bootstrap a real SPT dev environment via `quma setup`
dev-init:
    #!/usr/bin/env bash
    set -euo pipefail
    if [ -d "{{dev_dir}}/SPT" ]; then
        echo "Dev environment already exists at {{dev_dir}}/"
        echo "Run 'just dev-reset-db' to wipe the database, or 'just dev-clean' to start over."
        exit 0
    fi
    cargo run -- setup "{{dev_dir}}" --no-fika --no-modsync --admin-password devdevdev --dev --container-name "{{dev_container}}"

# Build and run the web server against the dev directory
dev-serve *ARGS: dev-init
    @echo "Dev server on port {{dev_port}} (container: {{dev_container}})"
    QUMA_SPT_DIR="{{dev_dir}}" QUMA_WEB_PORT="{{dev_port}}" cargo run -- serve {{ARGS}}

# Run any quma command against the dev directory
dev-cli *ARGS: dev-init
    QUMA_SPT_DIR="{{dev_dir}}" QUMA_WEB_PORT="{{dev_port}}" cargo run -- {{ARGS}}

# Install development tools (cargo-watch for auto-reload)
dev-install-tools:
    cargo install cargo-watch

# Auto-rebuild and restart the dev server on file changes
dev-watch *ARGS: dev-init
    @echo "Dev server on port {{dev_port}} (container: {{dev_container}})"
    QUMA_SPT_DIR="{{dev_dir}}" QUMA_WEB_PORT="{{dev_port}}" cargo watch -x 'run -- serve {{ARGS}}' -w src -w templates

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
    # Copy profile fixtures if any exist
    fixtures="dev/fixtures/profiles"
    target="{{dev_dir}}/SPT/user/profiles"
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

# Remove the dev directory and container entirely
dev-clean:
    #!/usr/bin/env bash
    set -euo pipefail
    container="{{dev_container}}"
    # Stop and remove the dev container if it exists
    if podman inspect "$container" &>/dev/null 2>&1; then
        podman stop "$container" 2>/dev/null || true
        podman rm "$container"
        echo "Container '$container' removed."
    elif docker inspect "$container" &>/dev/null 2>&1; then
        docker stop "$container" 2>/dev/null || true
        docker rm "$container"
        echo "Container '$container' removed."
    fi
    rm -rf "{{dev_dir}}"
    echo "Dev environment removed."

# Build the minimal headless container image
build-headless:
    podman build -t localhost/quma-headless:latest container/headless/
