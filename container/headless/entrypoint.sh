#!/bin/bash
set -euo pipefail

# Validate required env vars
for var in PROFILE_ID SERVER_URL SERVER_PORT UDP_PORT; do
    if [ -z "${!var:-}" ]; then
        echo "ERROR: Required environment variable $var is not set" >&2
        exit 1
    fi
done

# Clean up stale Xvfb lock files from previous runs (container restart)
rm -f /tmp/.X99-lock /tmp/.X11-unix/X99

# Start Xvfb — needed by wineboot, winetricks, and the game client
Xvfb :99 -screen 0 1024x768x24 -nolisten tcp &
XVFB_PID=$!

# Wait for Xvfb to be ready
for i in $(seq 1 10); do
    if xdpyinfo -display :99 >/dev/null 2>&1; then
        break
    fi
    sleep 0.5
done

# Initialize Wine prefix on first boot
if [ ! -f "$WINEPREFIX/system.reg" ]; then
    echo "Initializing Wine prefix at $WINEPREFIX..."
    wineboot --update
    echo "Installing vcrun2019..."
    winetricks -q vcrun2019 || echo "WARN: vcrun2019 returned $?, continuing anyway"
    echo "Installing dotnetdesktop8..."
    winetricks -q dotnetdesktop8 || echo "WARN: dotnetdesktop8 returned $?, continuing anyway"
    echo "Wine prefix initialized."
fi

# Wait for the SPT server to be reachable before launching.
# BepInEx plugins (SPT.Custom) make HTTP calls during static init —
# if the server isn't up yet, patches fail to load and Fika never registers.
echo "Waiting for SPT server at https://$SERVER_URL:$SERVER_PORT ..."
for i in $(seq 1 120); do
    if curl -sk --max-time 2 -H "responsecompressed: 0" "https://$SERVER_URL:$SERVER_PORT/launcher/ping" 2>/dev/null | grep -q pong; then
        echo "SPT server is ready."
        break
    fi
    if [ "$i" -eq 120 ]; then
        echo "ERROR: SPT server not reachable after 120 attempts" >&2
        exit 1
    fi
    sleep 2
done

# Tail BepInEx logs to stdout if the log file exists (or will exist)
if [ -d "/opt/tarkov/BepInEx" ]; then
    touch /opt/tarkov/BepInEx/LogOutput.log
    tail -f /opt/tarkov/BepInEx/LogOutput.log &
fi

# Launch the game client
exec wine /opt/tarkov/EscapeFromTarkov.exe \
    -batchmode \
    -nographics \
    -noDynamicAI \
    -token="$PROFILE_ID" \
    -config="{'BackendUrl':'https://$SERVER_URL:$SERVER_PORT','Version':'live'}"
