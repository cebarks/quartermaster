# Headless image entrypoint and display isolation

Each headless container starts its own `Xvfb -displayfd 3` and waits for its reported numeric display and live child before exporting `DISPLAY` to Wine. It does not reuse an inherited display or select a fixed display number. Xvfb's own allocation handles collisions, including X11 abstract sockets shared by containers using host networking. If Xvfb dies, stalls or reports an invalid display before readiness, Wine is not launched; the entrypoint checks Xvfb liveness again before starting EFT. This does not promise recovery if Xvfb dies after EFT starts.

On SIGTERM, the entrypoint exits with code 143. Its EXIT trap sends TERM to direct Wine children and their initial `setsid` process groups; after a bounded grace period it sends KILL to survivors. Xvfb remains available during that grace period, then the entrypoint stops its own Xvfb and log tail. It neither removes other instances' X sockets nor sends a machine-wide Wine kill. The runtime image explicitly installs `util-linux-core` for `setsid` and the external, negative-process-group-aware `/bin/kill`, in addition to Xvfb. A Wine process that fully detaches into a different session is outside the initial process-group guarantee.

## Native image verification

Build from `container/headless` with Podman, then test the immutable local image ID, not an old registry tag or a mounted replacement entrypoint:

```sh
podman build --layers --jobs=1 --pull=missing \
  --tag localhost/quma-headless:display-test \
  --file container/headless/Containerfile container/headless
image_id=$(podman image inspect localhost/quma-headless:display-test --format '{{.Id}}')
sh -n container/headless/entrypoint.sh
shellcheck container/headless/entrypoint.sh
python3 -B scripts/test-headless-entrypoint.py "$image_id" \
  --evidence-dir "$(mktemp -d)/fixtures"
```

The evidence directory argument must not already exist; retain the parent directory path if you need to inspect the fixture files, container logs and inspect JSON. The harness runs disposable PID1 containers with real Xvfb and synthetic Wine/game scripts. It checks concurrent display separation, X11 round trips while a child shuts down, failed Xvfb startup, wineboot cancellation, a stubborn child requiring bounded escalation, pre-`setsid` direct-child ownership, and Xvfb death before EFT launch. The pre-`setsid` hold is not a deterministic trap precisely between a shell fork and its `$!` assignment; that smaller interval remains reviewed rather than directly exercised. These fixtures do not certify real detached Wine descendants, live gameplay, raid saves or a deployed server.

The harness uses `--network=host` so X11 abstract sockets really can collide; it mounts only disposable fixture directories and disables SELinux relabeling on those mounts. Do not run it against an image or host you do not intend to expose to this test: host networking shares the host network namespace. Each run uses a unique `quma.xvfb-test` label and unique container names. Its `finally` handler retains fixture evidence and forcibly removes only containers whose names *and* ownership labels match that run (even after a failed assertion). A hard-killed harness can leave its own fixtures behind; inspect names and labels before any manual cleanup. Exploratory `--entrypoint /absolute/script` mode mounts a local script, but does not qualify the image entrypoint.

The headless-image workflow builds once in a `contents: read` job with checkout credentials disabled, checks the packaged group-signaling tools, runs this harness against its built image ID and retains fixture evidence. Pull requests stop there: that job has no package-write token. On non-PR events only, it exports the tested image as an OCI archive and passes its SHA-256 and immutable image ID as independent job outputs. A separate package-write job with no source checkout downloads the same-run artifact, verifies its bytes against the build job's SHA-256, loads it and compares the imported image ID to the build job's ID *before* registry login, tagging or push. A failed native gate, transfer or identity check blocks publication; the publisher does not rebuild Wine or the image. Main pushes retain the `latest` tag; release and manual runs retain the workflow's metadata-driven tags. The archive is retained for one day; it consumes Actions artifact storage and requires available runner disk and transfer capacity, so hosted qualification remains mandatory. Generated changelog entries come from commits; this document is not a deployment record.
