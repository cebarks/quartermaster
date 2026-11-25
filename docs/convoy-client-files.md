# Convoy client file policy

## Published contents

Convoy catalogs and both public archive routes distribute tracked `BepInEx/` files and **archive/package-tracked direct DLL children** of `EscapeFromTarkov_Data/Managed/`. Managed subdirectories, other extensions, untracked game assemblies and other game-root paths are not included. Existing bundle-manifest exports to `SPT[_Runtime]/user/cache/bundles/` are unchanged. Server mods, profiles and private server files are not client files.

Paths must use canonical forward slashes and exact root spelling; traversal, drive/absolute paths, empty/dot components, Windows device names and trailing-dot/space aliases are rejected. Downloads continue to use scoped layered reads (runtime upper, mod overlay, base), refusing server-target or escaping links. Rehash now reads those same layers for both supported client roots. Duplicate shared paths are written once; conflicting spellings/hashes fail archive construction.

Catalog/rehash cache filenames and archive policy identity are versioned so pre-change caches are not reused. Archive identity also includes tracked paths, hashes and sources, preventing a warm ID-only archive from hiding newly advertised dependencies. Existing mutation-driven invalidation and archive TTL remain. An eligible tracked BepInEx file or package Managed dependency without a checksum fails catalog generation and archive delivery (including cache hits) with the missing path; rehash the package before publishing. Incomplete inventories are not silently advertised as complete packages.

The setup ZIP remains independent of Convoy enablement and retains its separately configured pre-change filtering policy: tracked content outside server roots (including `winhttp.dll`, `doorstop_config.ini`, `user/mod-assets/` and Managed dependencies) is included by default, with non-essential filtering and explicit include/exclude overrides unchanged. This is not the narrower Convoy publication allowlist; it does not scan arbitrary directories. This ZIP is not a transactional installer: extracting it over a populated installation can overwrite local files and does not grant Convoy ownership. Matching existing files are usable but remain unowned in the updated client.

## Client ownership contract

The corresponding Convoy client validates the full advertised archive before mutation, rejects extra entries and unsafe/colliding paths, and checks existing destination components for symlinks/reparse points. State records distinguish owned files from matching pre-existing files. An unowned or locally modified assembly is not overwritten; a differing unowned dependency is a clear conflict. Existing BepInEx configuration files are preserved. Only unchanged owned files absent from every retained mod may be deleted. Shared dependencies remain while any selected mod needs them. Legacy state grants ownership only to BepInEx paths, never Managed assemblies.

Ordinary I/O failures roll file replacements/deletions back and state is atomically replaced after successful application. This is not a crash-durable multi-file journal or a defense against hostile concurrent local filesystem mutation. Close other installers during sync. Loaded Windows DLL backups may remain under unique `.convoy-old-*` names; no recursive cleanup claims unrelated files.

## Version skew and rollout

No capability negotiation or new authentication/protocol mechanism is introduced. **Upgrade Convoy clients first, restart them, then enable this publisher version.** Publish the new Convoy client as a distinct mod version using the existing BepInEx-only bootstrap/self-update path on the old publisher; do not bundle a dependency-bearing mod into that bootstrap operation. Verify all participants have upgraded before exposing new Managed paths.

Old clients accept game-root archive paths without the new ownership safeguards and can overwrite/adopt existing files. They also miss same-version manifest additions. Therefore a new publisher is not safe for uncontrolled old clients. New clients on an old publisher can operate on BepInEx-only archives, but reject old archives containing unadvertised Managed entries; do not perform dependency-bearing installations in the intermediate rollout window. There is no automatic enforcement of this ordering. A deployment needing mixed-client operation requires a separately reviewed capability policy, not an implicit fallback that omits dependencies.

The new client compares advertised file sets/hashes as well as mod version, so an already installed same-version mod acquires newly advertised dependencies after publisher upgrade. Release packaging/version assignment and deployment remain separate from source changes. Native Unity startup/raid behavior is not established by source tests or Mono loopback installation tests.
