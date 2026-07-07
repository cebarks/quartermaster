#!/bin/sh
set -e

export WINEPREFIX=/opt/wine-seed
export WINEARCH=win64
export WINEDEBUG=-all
export WINEDLLOVERRIDES="mscoree=d;mshtml=d"
export LD_LIBRARY_PATH=/opt/wine-cachyos/lib
export PATH="/opt/wine-cachyos/bin:$PATH"

# Start Xvfb for wine
Xvfb :99 -screen 0 1024x768x24 -nolisten tcp &
export DISPLAY=:99
sleep 1

# Create the prefix
wineboot --init
wineserver -k || true
sleep 1

# Patch: ProxyEnable=0 prevents Mono NullRef in AutoWebProxyScriptEngine
sed -i '/\[Software\\\\Microsoft\\\\Windows\\\\CurrentVersion\\\\Internet Settings\]/a "ProxyEnable"=dword:00000000' \
    /opt/wine-seed/user.reg

# Patch: winhttp=native,builtin for BepInEx doorstop injection
if grep -q '\[Software\\\\Wine\\\\DllOverrides\]' /opt/wine-seed/user.reg; then
    sed -i '/\[Software\\\\Wine\\\\DllOverrides\]/a "winhttp"="native,builtin"' /opt/wine-seed/user.reg
else
    printf '\n[Software\\\\Wine\\\\DllOverrides]\n"winhttp"="native,builtin"\n' >> /opt/wine-seed/user.reg
fi

# Strip the seed to just registry files — drive_c is 1.5GB of unnecessary
# Windows directory structure that wineboot recreates at runtime anyway
rm -rf /opt/wine-seed/drive_c /opt/wine-seed/dosdevices
echo "Wine prefix seeded at /opt/wine-seed (registry only)"
grep ProxyEnable /opt/wine-seed/user.reg
grep winhttp /opt/wine-seed/user.reg
du -sh /opt/wine-seed/

# Clean up Xvfb
kill %1 2>/dev/null || true
