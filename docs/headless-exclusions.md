# Headless exclusion reconciliation

Mod groups marked `exclude_headless` control tracked client files in each managed headless client's private native overlay. Exclusion never deletes shared player/server publications or per-client configuration. Existing private copies are verified and quarantined outside the game view; lower-only payloads are hidden through native copy-on-write overlay operations. Re-inclusion restores verified current published bytes rather than stale quarantined bytes, and unrelated files and unowned masks remain untouched.

## Managed lifecycle

The daemon prepares exclusions before initial configuration boot, subsequent first boot, explicit start/restart, convergence restarts, clean/crash recovery and eligible Ready-memory restarts. It reads a fresh fallible policy at each safe start. A policy, inspection, preparation or namespace failure prevents that start. Saving groups remains non-disruptive: a running client's loaded plugins are not hot-deleted and no restart is scheduled merely by saving policy.

The same per-client operation guard spans stop, stopped-state preparation and actual start. Blocking native reconciliation retains a guard clone even if its outer future is canceled, and cancellation never authorizes a later start. Scale/delete/rebuild acquire the bounded managed namespace in ascending order before destructive work and fail busy rather than bypassing another operation. Existing manual-stop and raid safety checks still apply. A rebuild is destructive to complete client overlays/Wine prefixes; it is not a substitute for exclusion reconciliation.

For native Podman containers whose first inspection has empty `Binds`/`Mounts`, only the actual successful managed creation path can supply opaque, single-use proof binding the exact create request, returned original container ID and durable backing identities. This permits preparation before the first boot; the ordinary existing-container inspector is not relaxed. Supervisor preparation uses the explicitly detected installation context, not a mutable config root or the location of an external config file.

### Overlay generations and destructive lifecycle operations

Active exclusion ownership is bound to the durable overlay root and upper directory identities plus independent directory-attached generation tokens, not to a container ID or a permanently reusable client index. Container-only/NUMA replacement retaining those directories keeps its ownership; ephemeral work-directory reinitialization does not retire it. Authorized rebuild, scale-down cleanup and selected-index deletion retire only the overlays they destroy. Persistent operation-lock inodes, quarantine bytes and private archived journals remain under `headless-policy`, outside the game view.

Before removal, teardown durably archives the journal and records retirement intent under the held operation guard. Unmount, evidence, removal or metadata failures stop the operation without starting replacements. An incomplete retirement blocks ordinary preparation and read-only checks. Retrying authorized teardown, or subsequent guarded convergence, can finish only that already-recorded retirement; a different replacement directory in its place refuses. No missing directory or changed mask alone grants retirement authority. Selected deletion keeps its configured selection available until teardown succeeds, so a failed teardown can be retried.

After successful removal and parent-directory sync, the active journal drops native mask ownership but retains logical path intent and verified historical hashes. Known removed/renamed lower payloads are hidden again in a fresh generation, even when no longer tracked; unknown changed retired payloads still refuse. Current exclusions reapply normally. Re-inclusion uses current verified publications, never quarantine or archived bytes; an unowned mask in the fresh upper is not adopted or removed.

Journal version 3 binds cryptographically random generation labels to the root and upper using the private versioned `user.quma.overlay-generation.v1` extended attribute. Create-only allocation and directory/parent sync precede journal authority and native edits. These labels survive removal of directory children and disappear with their own directory; recycled device/inode numbers alone never authorize deletion. A retry requires the original root proof and, if upper still exists, its original proof. A proven original root with an already-removed upper can finish automatically, as can acknowledgement after the entire root has been removed under durable retirement intent. A replacement root or upper, or missing, malformed or changed proof on a present owned directory, refuses without repairing ownership. Work-directory recreation remains independent.

The backing filesystem must support durable directory user xattrs. Unsupported metadata operations fail closed; there is no inode-only fallback. Interrupted initialization can resume while the journal is still unbound and no native ownership has been granted, retaining valid existing labels and creating only missing labels. Once bound, labels are never silently replaced or regenerated. `--check` only reads proof and never allocates labels, upgrades journals or creates state.

Legacy version-1/version-2 active or pending ownership lacks this durable proof and is conservatively refused, even when numeric directory identities match. Only unbound legacy journals or retired-only logical intent with no active generation, retirement or native mask authority can be accepted by a mutating operation after preflight; old proof is never fabricated. Legacy active/pending installations require separately authorized recovery preserving their data and evidence, not an automatic upgrade, teardown retry or deletion of journals. Unsupported/inconsistent metadata and previously destroyed overlays with stale ownership but **no durable retirement intent** also refuse. Keep the updated daemon/CLI together after version-3 state is written; older readers reject that version. Do not delete policy journals, xattrs or lock files to bypass refusal.

These hooks require the updated daemon. Running the offline command or installing a CLI does not upgrade an already-running older daemon. Private fixture coverage is not evidence of production deployment or successful player raid entry/extraction/save.

## Offline command

```sh
quma --quma-dir /path/to/quma headless reconcile-exclusions 1 --check
quma --quma-dir /path/to/quma headless reconcile-exclusions 1
```

Client indices are positive. The command prints the actual reconciliation report as JSON. `--check` plans changes without creating policy state, locks, directories, masks or mounts. Both modes require inspection of the original managed `fika-headless-N` container; running, paused, restarting, unknown or uninspectable clients refuse with a nonzero exit status. This command never stops, starts, creates, scales or deletes a container. Stop the client using its normal authorized lifecycle before applying. An error must prevent any subsequent start.

The offline path does not load Fika API tokens, profiles, Forge clients or normal configuration, migrate the database, drain queues or run convergence. It requires an existing database and the original native game base/private upper/work binding. Additional/unknown game mounts, changed ownership, aliased paths and mismatching container identities refuse.

## Read-only database contract

The reader never runs SQL writes, migrations or RBAC synchronization. It supports ordinary DELETE-journal databases without a rollback journal, and WAL databases only when both the original WAL and SHM already exist as regular, same-user, non-group/world-writable, single-link files in a non-aliased, same-user, non-group/world-writable directory. Committed WAL data is read through SQLite's normal reader-lock/snapshot protocol, not ignored with `immutable=1`.

SQLite may update existing SHM reader-lock/WAL-index bookkeeping. The promise is **no Quma database/schema/policy/payload writes or source-side sidecar creation**, not byte-for-byte immutability of SQLite's live shared-memory bookkeeping. Missing WAL/SHM, rollback journals, aliases and unsupported namespace states fail closed. This intentionally includes a clean, closed WAL-mode database whose sidecars have been removed; arrange an independently authorized, supported SQLite reader context rather than enabling immutable mode or asking this command to repair it.

The database and installation namespace must remain stable during the operation: no out-of-band rename, unlink, replacement, journal-mode transition, sidecar removal or direct runtime lifecycle action. Ordinary SQLite concurrent writers are supported. Advisory locks coordinate cooperating Quma operations, not hostile writers running as the same Unix account. The held guard detects directory/file replacement and unsafe permission changes before guarded work; it cannot defeat arbitrary concurrent same-account namespace changes.

## Limitations

- The offline adapter does not start clients or install/upgrade daemon hooks. Use the updated daemon for managed lifecycle preparation.
- The offline command refuses a never-started `:O` container with empty `Binds`/`Mounts`: it has no owned creation proof. The normal managed creation path supplies its own trustworthy proof; never boot merely to bypass offline inspection refusal.
- Kernel/FUSE busy detection cannot see every hidden PID/mount namespace. Managed stopped-container inspection is mandatory, not replaced by filesystem checks.
- Global `--quma-dir` is forwarded by the headless dispatch; `QUMA_DIR` and the deprecated `--spt-dir` fallback remain supported.
- Independent implementation review, exact-artifact native validation and separately authorized deployment/gameplay checks remain distinct gates. A successful command is not evidence of player handoff, raid entry, play, extraction or saving.
