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

dev_dir := ".dev"

# Create a local fake SPT directory for development
dev-init:
    #!/usr/bin/env bash
    set -euo pipefail
    if [ -d "{{dev_dir}}/SPT" ]; then
        echo "Dev environment already exists at {{dev_dir}}/"
        echo "Run 'just dev-reset-db' to wipe the database, or 'just dev-clean' to start over."
        exit 0
    fi
    echo "Creating dev SPT directory at {{dev_dir}}/ ..."
    mkdir -p "{{dev_dir}}/SPT/SPT_Data/configs"
    mkdir -p "{{dev_dir}}/SPT/user/mods"
    mkdir -p "{{dev_dir}}/BepInEx/plugins"
    touch "{{dev_dir}}/SPT/SPT.Server.exe"
    cat > "{{dev_dir}}/SPT/SPT_Data/configs/core.json" <<'EOF'
    {
      "projectName": "SPT",
      "compatibleTarkovVersion": "0.16.9.40087",
      "serverName": "SPT Server",
      "profileSaveIntervalSeconds": 60
    }
    EOF
    cat > "{{dev_dir}}/SPT/SPT.Server.deps.json" <<'EOF'
    {"libraries":{"SPT.Server/4.0.13-RELEASE+dev.00000000":{}}}
    EOF
    cat > "{{dev_dir}}/quartermaster.toml" <<'EOF'
    web_bind = "127.0.0.1"
    web_port = 9190
    queue_changes = false
    auto_start_server = false
    proxy_enabled = false
    tls_enabled = false
    EOF
    echo "Dev environment ready. Run 'just dev-serve' to start the web UI."

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

# Remove the dev directory entirely
dev-clean:
    rm -rf "{{dev_dir}}"
    echo "Dev environment removed."
