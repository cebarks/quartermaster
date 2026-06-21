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

lint: fmt clippy

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

# Bootstrap a real SPT dev environment via `quma setup`
dev-init:
    #!/usr/bin/env bash
    set -euo pipefail
    if [ -d "{{dev_dir}}/SPT" ]; then
        echo "Dev environment already exists at {{dev_dir}}/"
        echo "Run 'just dev-reset-db' to wipe the database, or 'just dev-clean' to start over."
        exit 0
    fi
    cargo run -- setup "{{dev_dir}}" --no-fika --no-modsync --admin-password devdevdev --no-forge-token --dev

# Build and run the web server against the dev directory
dev-serve *ARGS: dev-init
    QUMA_SPT_DIR="{{dev_dir}}" cargo run -- serve {{ARGS}}

# Run any quma command against the dev directory
dev-cli *ARGS: dev-init
    QUMA_SPT_DIR="{{dev_dir}}" cargo run -- {{ARGS}}

# Wipe the dev database (keeps config and SPT structure)
dev-reset-db:
    rm -f "{{dev_dir}}/quartermaster.db" "{{dev_dir}}/quartermaster.db-journal" "{{dev_dir}}/quartermaster.db-wal"
    echo "Dev database wiped."

# Remove the dev directory and container entirely
dev-clean:
    #!/usr/bin/env bash
    set -euo pipefail
    # Stop and remove the dev container if it exists
    if podman inspect spt-server-dev &>/dev/null 2>&1; then
        podman stop spt-server-dev 2>/dev/null || true
        podman rm spt-server-dev
        echo "Container 'spt-server-dev' removed."
    elif docker inspect spt-server-dev &>/dev/null 2>&1; then
        docker stop spt-server-dev 2>/dev/null || true
        docker rm spt-server-dev
        echo "Container 'spt-server-dev' removed."
    fi
    rm -rf "{{dev_dir}}"
    echo "Dev environment removed."
