#!/bin/sh
set -e

SERVER_DIR=/opt/server/SPT

_pid=""
on_term() { [ -n "$_pid" ] && kill -TERM "$_pid" 2>/dev/null; }
trap on_term TERM INT

cd "$SERVER_DIR"
dotnet SPT.Server.dll &
_pid=$!
wait "$_pid"
