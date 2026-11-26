# Convoy client file policy

## Published contents

Convoy v3 publishes tracked `BepInEx/` files (excluding private Convoy state and staging), archive-tracked direct DLLs under `EscapeFromTarkov_Data/Managed/`, and eligible declared, tracked SPT bundle cache files. Paths and sources are validated through scoped layered reads. Conflicting identities, missing current baselines and unsafe files prevent publication rather than advertising incomplete packages.

The public `/quma/convoy/v3/catalog` has `protocol: 3` and describes one immutable generation. Each asset has `path`, `length` (a nonnegative 64-bit value), and `xxh3_128` (seed-zero XXH3-128, 32 lowercase hex digits in unsigned high-to-low order). Clients request raw objects from `/quma/convoy/v3/generations/{generation}/objects/{digest}`; there is no archive generation, download or extraction for sync/repair. Strong catalog/object ETags use the new identities. Validated interruptions resume with `Range`/`If-Range`; complete length and checksum verification precedes apply. Matching local bytes can be reused; a no-op sync makes no object GETs. Selection remains per mod.

XXH3-128 is non-cryptographic: it detects accidental corruption, not deliberate collisions or an untrusted publisher. TLS and publisher trust remain separate. No SHA-256 or XXH3-64 compatibility hash is calculated in the new delivery/tracking flow.

The setup ZIP remains independent of Convoy enablement and retains its separately configured filtering policy: tracked content outside server roots (including `winhttp.dll`, `doorstop_config.ini`, `user/mod-assets/` and Managed dependencies) is included by default, with non-essential filtering and explicit include/exclude overrides unchanged. It is not the narrower Convoy allowlist and does not scan arbitrary directories. This ZIP is not a transactional installer: extracting over a populated installation can overwrite local files and does not grant Convoy ownership.

## Client ownership contract

The client validates the complete manifest before selection or mutation and rejects unsafe/colliding paths and symlink/reparse destinations. Matching pre-existing files stay unowned. Differing plugin DLL replacement requires explicit consent and a recovery backup of the exact original bytes; other conflicts remain errors. Existing BepInEx configurations are preserved. Only unchanged owned files absent from every retained mod may be deleted; shared dependencies remain while needed.

Legacy SHA-256/XXH3-64 state remains pending and grants no deletion/replacement authority. The player review displays path, old ownership and observed/desired identities. Approval trusts current bytes, not historical unchangedness, and preserves original ownership and checksum evidence. Differing plugin DLLs require separate backed-up replacement consent. Missing, excluded or removed-mod files stay pending and untouched. Configs are preserved regardless of approval. Changes between display and apply require replanning. Null legacy ownership retains only its prior BepInEx meaning, never Managed ownership.

Ordinary I/O failures roll mutations back; state is atomically replaced after successful application. This is not a crash-durable multi-file journal or a defense against hostile concurrent local filesystem mutation. Close other installers during sync. Loaded Windows DLL backups may remain under unique `.convoy-old-*` names; no recursive cleanup claims unrelated files.

## Paired upgrade and rollback

Close the game and upgrade the standalone Convoy client and Quartermaster as a v3 pair. See [Convoy installation](https://github.com/cebarks/convoy#installation). Do not rely on self-update across this boundary. An enabled new server returns `410 Gone` on v2 catalog/object and old catalog/ZIP routes; a new client refuses an old server without changing files or attempting legacy delivery. Disabled delivery returns `503`. Setup/invite ZIP behavior remains independent. This source change does not publish a v3 release or assert that Forge `2806` carries it.

Before cutover, back up old binaries, server configuration, compatible database and publisher object store, and each client's plugin/configuration/ownership state. Roll back both binaries and compatible data together; old binaries are not guaranteed to understand migrated state. Preserve separately any client edits since the backup. Expect downtime until selected legacy tracked assets have explicit baselines and the new generation reaches **Ready**.

### Explicit server rebaseline

Database migration preserves old values and algorithm provenance in `legacy_file_hashes` and marks those records pending. It leaves genuinely unknown NULL baselines unknown. Startup, setup and layout migration do not trust current bytes automatically. Inspect `quma status --json` (`integrity.unverified_files` includes IDs, paths and `legacy`/`unknown` reasons), or the file/integrity pages. Back up and inspect the selected current files before running:

```bash
quma --quma-dir /path/to/installation rebaseline --file-id 42 --file-id 43 --trust-current
```

`--trust-current` acknowledges a new baseline, not verification against the old checksum. No implicit bulk action is available. Only successfully read selected records change; missing, unreadable, hidden, unsafe or concurrently changed files stay pending. Partial success is retained; retry unresolved IDs after fixing their cause. Disabled mod/addon files resolve through the stash; enabled files respect overlay visibility. Historical evidence and unselected records remain intact.

A pending tracked client asset prevents the whole generation from publishing. Server-only pending records stay unverified in health but do not block unrelated client publication. **Rehash and republish** does not bypass this requirement; it rehashes already-current baselines following out-of-band edits. Wait for **Ready** and consult the server log on **Failed**.

### Capacity and retained objects

Budget `[convoy].max_store_bytes` and physical disk for retained legacy objects, active/pinned objects and candidate bytes, in addition to the original sources. First publication reads and stages the full unique content; reopening a v3 receipt verifies stored bytes before serving. Legacy receipts are not authenticated or served as v3. Capacity failure preserves old durable data and does not increase the limit.

Cleanup never sweeps files merely because they look like digests. Unknown startup objects and SHA-named legacy bytes are retained. In-process retirement holds verified object identity handles, preserves pinned requests, and retains unverifiable replacements. Retained handles are bounded to 64; excess objects remain and count against capacity. Repeated restarts or large retirements may therefore leave storage requiring a separately reviewed operator cleanup. Do not delete retained data just to make migration fit without reviewing ownership and rollback needs.

For an unpublished local pair, build against legitimate SPT references with `DeployAfterBuild=false`. Package only `Convoy.dll` and `System.IO.Hashing.dll` from the client's Release output under `BepInEx/plugins/Convoy/`. Do not package game reference assemblies. Release/tag/deployment is a separate action. Linux/Mono integration tests do not qualify native Windows/Unity startup or loaded-DLL sharing.
