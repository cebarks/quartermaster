#!/bin/sh
set -e

EFT_DIR=/opt/tarkov
WINE_LOG="${EFT_DIR}/wine.log"
BEPINEX_LOG="${EFT_DIR}/BepInEx/LogOutput.log"

divider() { printf '\n%.0s' 1; printf '=%.0s' $(seq 1 60); printf '\n\n'; }

# Forward SIGTERM to wine so shutdown is clean
_eft_pid=""
_boot_pid=""
_xvfb_pid=""
_tail_pid=""
_display_file=""
_launching=""
_term_pending=""
on_term() {
    echo "Received SIGTERM — forwarding to EFT"
    # A trap may run between the asynchronous fork and recording $!.
    # Finish recording ownership before EXIT cleanup inspects the child.
    if [ -n "$_launching" ]; then
        _term_pending=1
        return
    fi
    exit 143
}
on_exit() {
    _status=$?
    trap '' TERM INT
    # Wine may launch a separate game child. Signal only our own sessions,
    # keeping X alive for bounded cleanup before stopping auxiliaries.
    for _pid in "$_boot_pid" "$_eft_pid"; do
        [ -n "$_pid" ] || continue
        # setsid may not have created the group yet: cover the direct child
        # as well, never the inherited (PID1/auxiliary) process group.
        /bin/kill -TERM -- "-$_pid" "$_pid" 2>/dev/null || true
        for _attempt in $(seq 1 50); do
            if ! /bin/kill -0 -- "-$_pid" 2>/dev/null &&
               ! kill -0 "$_pid" 2>/dev/null; then
                break
            fi
            sleep 0.1
        done
        if /bin/kill -0 -- "-$_pid" 2>/dev/null || kill -0 "$_pid" 2>/dev/null; then
            echo "WARN: Wine process group $_pid exceeded shutdown grace; sending SIGKILL" >&2
            /bin/kill -KILL -- "-$_pid" "$_pid" 2>/dev/null || true
        fi
        # Do not wait here: cleanup must stay bounded even if a killed child
        # cannot be reaped immediately. PID1 is exiting after auxiliary cleanup.
    done
    for _pid in "$_tail_pid" "$_xvfb_pid"; do
        [ -z "$_pid" ] || kill -TERM "$_pid" 2>/dev/null || true
    done
    [ -z "$_display_file" ] || rm -f -- "$_display_file"
    divider
    echo "entrypoint exited (code $_status)"
    divider
}
trap on_term TERM INT
trap on_exit EXIT

echo "quma-headless starting"

# Host networking shares X11's abstract sockets even with separate /tmp dirs.
# Use the display our own Xvfb bound, never a pre-existing display or stale lock.
echo "Starting Xvfb..."
_display_file=$(mktemp)
Xvfb -displayfd 3 -screen 0 1024x768x24 -ac +extension GLX +render -noreset -nolisten tcp \
    3>"$_display_file" &
_xvfb_pid=$!
_display=""
for _attempt in $(seq 1 100); do
    if ! kill -0 "$_xvfb_pid" 2>/dev/null; then
        echo "ERROR: Xvfb exited before reporting its display" >&2
        exit 1
    fi
    if IFS= read -r _display < "$_display_file"; then
        break
    fi
    _display=""
    sleep 0.1
done
case $_display in
    ''|*[!0-9]*) echo "ERROR: Xvfb did not report a valid display" >&2; exit 1 ;;
esac
if ! kill -0 "$_xvfb_pid" 2>/dev/null; then
    echo "ERROR: Xvfb exited during startup" >&2
    exit 1
fi
rm -f -- "$_display_file"
_display_file=""
export DISPLAY=":$_display"
echo "Xvfb ready on $DISPLAY"

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
_launching=1
setsid /opt/wine-cachyos/bin/wineboot --update &
_boot_pid=$!
_launching=""
[ -z "$_term_pending" ] || exit 143
wait "$_boot_pid" || echo "WARN: wineboot exited $? (non-fatal)"
_boot_pid=""
/opt/wine-cachyos/bin/wineserver -k 2>/dev/null || true
sleep 1

echo "Wine prefix ready"
echo "Server: https://${SERVER_URL:-host.containers.internal}:${SERVER_PORT:-6969}"
echo "Profile: ${PROFILE_ID:-(none)}"

divider

# Run headless client — redirect wine/Unity output to file,
# surface BepInEx logs in podman logs instead
cd "$EFT_DIR"
BACKEND="https://${SERVER_URL:-host.containers.internal}:${SERVER_PORT:-6969}"
if ! kill -0 "$_xvfb_pid" 2>/dev/null; then
    echo "ERROR: Xvfb exited before EFT launch" >&2
    exit 1
fi
_launching=1
setsid /opt/wine-cachyos/bin/wine EscapeFromTarkov.exe \
    -batchmode -nographics -noDynamicAI \
    -token="${PROFILE_ID}" \
    -config="{\"BackendUrl\":\"${BACKEND}\"}" \
    > "$WINE_LOG" 2>&1 &
_eft_pid=$!
_launching=""
[ -z "$_term_pending" ] || exit 143
echo "EFT PID is $_eft_pid"

# Surface BepInEx logs in podman logs
tail -f -n 0 "$BEPINEX_LOG" 2>/dev/null &
_tail_pid=$!

# Block until EFT exits
wait "$_eft_pid" || true
