#!/bin/bash
set -euo pipefail

# Validate required env vars
for var in PROFILE_ID SERVER_URL SERVER_PORT UDP_PORT; do
    if [ -z "${!var:-}" ]; then
        echo "ERROR: Required environment variable $var is not set" >&2
        exit 1
    fi
done

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
    winetricks -q vcrun2019 || { echo "ERROR: Failed to install vcrun2019" >&2; exit 1; }
    echo "Installing dotnetdesktop8..."
    winetricks -q dotnetdesktop8 || { echo "ERROR: Failed to install dotnetdesktop8" >&2; exit 1; }
    echo "Wine prefix initialized."
fi

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
