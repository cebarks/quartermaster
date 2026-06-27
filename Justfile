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

# --- Container Image ---

# Build the headless client container image
build-headless:
    podman build -t quma-headless:latest container/headless/

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

# Install development tools (cargo-watch for auto-reload)
dev-install-tools:
    cargo install cargo-watch

# Auto-rebuild and restart the dev server on file changes
dev-watch *ARGS: dev-init
    QUMA_SPT_DIR="{{dev_dir}}" cargo watch -x 'run -- serve {{ARGS}}' -w src -w templates

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
