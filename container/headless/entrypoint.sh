#!/bin/sh
set -e

divider() { printf '\n%.0s' 1; printf '=%.0s' $(seq 1 60); printf '\n\n'; }
trap 'divider; echo "entrypoint exited (code $?)"; divider' EXIT

echo "quma-headless starting"

# Virtual display
echo "Starting Xvfb..."
Xvfb :99 -screen 0 1024x768x24 -nolisten tcp &
export DISPLAY=:99

# Wine prefix — /.wine when overlay-mounted by quma, /tmp/.wine otherwise
export WINEPREFIX="${WINEPREFIX:-/.wine}"
export WINEARCH=win64
export WINEDEBUG=-all

# BepInEx injection + suppress mono/gecko install dialogs
export WINEDLLOVERRIDES="winhttp=n,b;mscoree=d;mshtml=d"

# Prevent Mono from probing for system proxy (causes NullRef in AutoWebProxyScriptEngine)
export no_proxy="*"
export NO_PROXY="*"

# Import system CA certs into Mono's trust store so TLS works
if [ -f /etc/ssl/certs/ca-bundle.crt ]; then
    cert-sync --quiet /etc/ssl/certs/ca-bundle.crt 2>/dev/null || true
fi

# Sync primitives: ntsync auto-detected via /dev/ntsync device
if [ -c /dev/ntsync ]; then
    echo "ntsync: available"
else
    echo "WARN: /dev/ntsync not available — no ntsync, check host kernel 6.14+"
    [ "${ESYNC}" = "true" ] && export WINEESYNC=1 && echo "esync: enabled"
    [ "${FSYNC}" = "true" ] && export WINEFSYNC=1 && echo "fsync: enabled"
fi

# Initialize wine prefix from pre-seeded image prefix
if [ ! -f "$WINEPREFIX/user.reg" ]; then
    echo "Seeding wine prefix from image..."
    cp -a /opt/wine-seed/* "$WINEPREFIX/" 2>/dev/null || true
    cp -a /opt/wine-seed/.* "$WINEPREFIX/" 2>/dev/null || true
fi
echo "Updating wine prefix..."
/opt/wine-cachyos/bin/wineboot --update || echo "WARN: wineboot exited $? (non-fatal)"
/opt/wine-cachyos/bin/wineserver -k 2>/dev/null || true
sleep 1

echo "Wine prefix ready"
echo "Server: https://${SERVER_URL:-host.containers.internal}:${SERVER_PORT:-6969}"
echo "Profile: ${PROFILE_ID:-(none)}"

divider

# Run headless client
cd /opt/tarkov
BACKEND="https://${SERVER_URL:-host.containers.internal}:${SERVER_PORT:-6969}"
exec /opt/wine-cachyos/bin/wine EscapeFromTarkov.exe \
    -batchmode -nographics -noDynamicAI \
    -token="${PROFILE_ID}" \
    -config="{\"BackendUrl\":\"${BACKEND}\"}"
