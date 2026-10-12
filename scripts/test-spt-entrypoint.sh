#!/bin/sh
set -eu

image=${1:-localhost/quma-spt-server:4.1.6}
root=$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd)
fixture=$(mktemp -d)
trap 'rm -rf "$fixture"' EXIT HUP INT TERM
mkdir -p "$fixture/bin" "$fixture/server"
printf '#!/bin/sh\nprintf "launched:%%s:%%s\\n" "$PWD" "$1"\n' > "$fixture/bin/dotnet"
chmod +x "$fixture/bin/dotnet"

run_entrypoint() {
    podman run --rm --pull=never --entrypoint /bin/sh \
        -e PATH=/test-bin:/usr/bin \
        -v "$fixture/server:/opt/server:ro,Z" \
        -v "$fixture/bin:/test-bin:ro,Z" \
        -v "$root/container/spt-server/entrypoint.sh:/test-entrypoint.sh:ro,Z" \
        "$image" /test-entrypoint.sh
}

for runtime in SPT_Runtime SPT; do
    mkdir -p "$fixture/server/$runtime/SPT_Data/configs"
    touch "$fixture/server/$runtime/SPT.Server.deps.json" \
        "$fixture/server/$runtime/SPT_Data/configs/core.json" \
        "$fixture/server/$runtime/SPT.Server.Linux.dll"
    output=$(run_entrypoint)
    test "$output" = "launched:/opt/server/$runtime:SPT.Server.Linux.dll"
    printf 'PASS: %s launch without apphost execute permissions\n' "$runtime"
    rm -rf "$fixture/server/$runtime"
done

for runtime in SPT_Runtime SPT; do
    mkdir -p "$fixture/server/$runtime/SPT_Data/configs"
    touch "$fixture/server/$runtime/SPT.Server.deps.json" \
        "$fixture/server/$runtime/SPT_Data/configs/core.json" \
        "$fixture/server/$runtime/SPT.Server.Linux.dll"
done
if run_entrypoint > "$fixture/output" 2>&1; then
    printf 'FAIL: ambiguous runtimes were accepted\n' >&2
    exit 1
fi
rg -q 'ambiguous' "$fixture/output"
printf 'PASS: ambiguous runtimes rejected\n'
rm -rf "$fixture/server/SPT_Runtime" "$fixture/server/SPT"
if run_entrypoint > "$fixture/output" 2>&1; then
    printf 'FAIL: missing runtime was accepted\n' >&2
    exit 1
fi
printf 'PASS: missing runtime rejected\n'
