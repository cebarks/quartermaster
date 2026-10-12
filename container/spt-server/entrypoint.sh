#!/bin/sh
set -eu

has_server() {
    [ -f "$1/SPT.Server.deps.json" ] && [ -f "$1/SPT_Data/configs/core.json" ]
}

if has_server /opt/server/SPT_Runtime; then
    if has_server /opt/server/SPT; then
        printf 'ambiguous SPT installation: both SPT_Runtime and SPT contain server markers\n' >&2
        exit 1
    fi
    SERVER_DIR=/opt/server/SPT_Runtime
elif has_server /opt/server/SPT; then
    SERVER_DIR=/opt/server/SPT
else
    printf 'No valid SPT server found under /opt/server/SPT_Runtime or /opt/server/SPT\n' >&2
    exit 1
fi

cd "$SERVER_DIR"
exec dotnet SPT.Server.Linux.dll
