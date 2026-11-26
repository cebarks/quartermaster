# Convoy client file policy

## Published contents

Convoy v2 publishes tracked `BepInEx/` files (excluding private Convoy state and staging), archive-tracked direct DLLs under `EscapeFromTarkov_Data/Managed/`, and eligible declared SPT bundle cache files. Paths and sources are validated through scoped layered reads. Conflicting identities, missing tracked checksums and unsafe files prevent publication rather than advertising incomplete packages.

The public `/quma/convoy/v2/catalog` describes one immutable, verified generation. Clients request raw SHA-256/64-bit-length objects from that generation only; there is no archive generation, download, or extraction for sync or repair. A validated interruption can resume with `Range` and `If-Range`. Matching local bytes can be reused; a no-op sync makes no object GETs. Required and optional selections are still per mod.

The setup ZIP remains independent of Convoy enablement and retains its separately configured pre-change filtering policy: tracked content outside server roots (including `winhttp.dll`, `doorstop_config.ini`, `user/mod-assets/` and Managed dependencies) is included by default, with non-essential filtering and explicit include/exclude overrides unchanged. This is not the narrower Convoy publication allowlist; it does not scan arbitrary directories. This ZIP is not a transactional installer: extracting it over a populated installation can overwrite local files and does not grant Convoy ownership. Matching existing files are usable but remain unowned in the updated client.

## Client ownership contract

The corresponding Convoy client validates the complete accepted file plan before mutation, rejects extra entries and unsafe/colliding paths, and checks existing destination components for symlinks/reparse points. State records distinguish owned files from matching pre-existing files. An unowned or locally modified assembly is not overwritten; a differing unowned dependency is a clear conflict. Existing BepInEx configuration files are preserved. Only unchanged owned files absent from every retained mod may be deleted. Shared dependencies remain while any selected mod needs them. Legacy state grants ownership only to BepInEx paths, never Managed assemblies.

Ordinary I/O failures roll file replacements/deletions back and state is atomically replaced after successful application. This is not a crash-durable multi-file journal or a defense against hostile concurrent local filesystem mutation. Close other installers during sync. Loaded Windows DLL backups may remain under unique `.convoy-old-*` names; no recursive cleanup claims unrelated files.

## Paired upgrade and rollback

With the game closed, install a standalone Convoy v2 package using the [Convoy installation instructions](https://github.com/cebarks/convoy#installation) and upgrade Quartermaster as a pair. Do not rely on legacy self-update to cross this boundary. An enabled new server returns `410 Gone` on the old catalog and both ZIP routes, and a new client refuses an old server without changing files or trying legacy downloads. Disabled delivery returns `503`. The setup and invite ZIPs and Forge `2806` bootstrap remain independent; this source change does not assert that the Forge entry has a v2 package.

Before cutover, back up the previous binaries, server configuration and compatible Quartermaster database, and client plugin/configuration/ownership state. Roll back both ends and restore compatible data together; older software is not guaranteed to read the new state. Preserve separately any client files modified since the backup.

Budget `[convoy].max_store_bytes` and physical free space for the immutable object store as well as the source files, including changed bytes for the candidate alongside the active/pinned generation. First publication must copy and hash the full unique content; reopening an existing publication verifies its stored bytes before serving. Allow client disk for both transfer staging and temporary application/rollback copies. Managed changes automatically republish; for out-of-band edits or missing checksums use **Rehash and republish**. That queues background work; wait for **Ready** and consult the server log if it shows **Failed**. No-op managed publication can reuse verified objects.

For an unpublished local pair, build against legitimate SPT references with `DeployAfterBuild=false`. Package only `Convoy/bin/Release/Convoy.dll` and `Convoy/bin/Release/System.IO.Hashing.dll` under `BepInEx/plugins/Convoy/`. Do not package game reference assemblies. Naming/tagging/publishing a distinct release is a separate approval-gated action (the plugin version is derived from Git tags).
