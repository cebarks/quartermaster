#!/usr/bin/env bash
set -euo pipefail

ERRORS=0

echo "=== Logging Convention Check ==="

# Rule 1: error!() without err field (WARN — some error! calls are for conditions, not caught errors)
echo ""
echo "--- Checking: error!() without err field ---"
# Find error! calls, exclude lines that contain 'err' or 'err ='
VIOLATIONS=$(rg -n 'error!\(' src/ --glob '*.rs' | rg -v 'err\s*=' | rg -v '//.*error!' || true)
if [ -n "$VIOLATIONS" ]; then
    echo "WARN: error!() without err field (review — some may be intentional condition-based errors):"
    echo "$VIOLATIONS"
    # Don't increment ERRORS — this is advisory, not blocking
else
    echo "PASS"
fi

# Rule 2: No bare 'id =' in logging macros within mod-related files (should be mod_id, user_id, etc.)
echo ""
echo "--- Checking: bare 'id' field in logging macros in mod-related files ---"
# Look for logging macros (debug!, info!, warn!, error!) that contain bare 'id ='
# This is more precise than checking all 'id =' in the file (which catches SQL)
VIOLATIONS=$(rg -n '(debug!|info!|warn!|error!)\(.*id\s*=' src/ops.rs src/forge/ src/web/handlers/mods.rs src/db/mods.rs --glob '*.rs' 2>/dev/null | rg -v 'mod_id|user_id|version_id|forge_id|raid_id|request_id|task_id|csrf' || true)
if [ -n "$VIOLATIONS" ]; then
    echo "WARN: bare 'id' field in logging (should be mod_id, user_id, etc.):"
    echo "$VIOLATIONS"
    ERRORS=$((ERRORS + 1))
else
    echo "PASS"
fi

# Rule 3: No info! in hot-path modules
# Note: some info! in these files may be valid lifecycle events (connection open/close, initialization)
# This check allows known acceptable patterns and focuses on request-handling hot-paths
echo ""
echo "--- Checking: info!() in hot-path modules ---"
# Check for info! calls, excluding known acceptable lifecycle events:
# - WebSocket connection lifecycle (connected/disconnected)
# - One-time initialization events (auto-created accounts)
# Reads each file's info!() blocks and checks if the full invocation contains
# an excluded message pattern. This handles multi-line macro calls.
VIOLATIONS=""
for file in src/web/proxy.rs src/web/proxy_ws.rs src/web/sse.rs; do
    [ -f "$file" ] || continue
    # Extract line numbers of info!( calls
    while IFS=: read -r line_num _; do
        # Read from that line until the closing );
        block=$(sed -n "${line_num},/);/p" "$file")
        if ! echo "$block" | rg -q 'WebSocket proxy (connected|disconnected)|auto-created.*account'; then
            echo "${file}:${line_num}:$(sed -n "${line_num}p" "$file")"
            VIOLATIONS="found"
        fi
    done < <(rg -n 'info!\(' "$file" 2>/dev/null || true)
done
if [ -n "$VIOLATIONS" ]; then
    echo "FAIL: info!() in hot-path modules (should be debug! or trace!):"
    echo "$VIOLATIONS"
    ERRORS=$((ERRORS + 1))
else
    echo "PASS"
fi

# Rule 4: Use 'err' not 'error' as field name
echo ""
echo "--- Checking: 'error =' field name (should be 'err =') ---"
VIOLATIONS=$(rg -n 'error\s*=' src/ --glob '*.rs' | rg '(warn!|error!|info!|debug!)' | rg -v 'err\s*=' | rg -v '//.*error' | rg -v 'WebError' | rg -v 'map_err' || true)
if [ -n "$VIOLATIONS" ]; then
    echo "WARN: 'error =' field name (prefer 'err ='):"
    echo "$VIOLATIONS"
    ERRORS=$((ERRORS + 1))
else
    echo "PASS"
fi

echo ""
if [ "$ERRORS" -gt 0 ]; then
    echo "=== $ERRORS violation(s) found ==="
    exit 1
else
    echo "=== All checks passed ==="
fi
