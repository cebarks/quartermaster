#!/usr/bin/env bash
# Quartermaster client setup for an existing matching SPT installation.
# Run: bash quma-bootstrap.sh --spt-dir /path/to/SPT --dry-run
# Then rerun without --dry-run and confirm, or use --yes for an approved plan.
set -euo pipefail
if ! command -v python3 >/dev/null 2>&1; then
    printf '%s\n' 'ERROR: Python 3 is required. This script does not install system dependencies.' >&2
    exit 1
fi
# FD3 preserves confirmation input while stdin carries the embedded program.
python3 - "$@" 3<&0 <<'QUMA_BOOTSTRAP_PYTHON'
__QUMA_PYTHON__
QUMA_BOOTSTRAP_PYTHON
