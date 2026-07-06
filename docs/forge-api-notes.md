# SPT Forge API Notes

Undocumented behaviors and quirks observed while building the Forge client (`src/forge/`).

Base URL: `https://forge.sp-tarkov.com/api/v0`

## `fika_compatibility` type mismatch

The same field name uses two different JSON representations depending on the parent object:

| Context | Type | Values |
|---------|------|--------|
| Mod object (`ForgeMod`) | boolean | `true` / `false` |
| Version object (`ForgeVersion`) | string enum | `"compatible"` / `"incompatible"` / `"unknown"` |

Additionally, `false` on mod objects means "not assessed" (maps to `Unknown`), not "confirmed incompatible".

Quartermaster handles this with a custom `FikaCompat` deserializer that accepts both representations via `#[serde(untagged)]` on a raw enum. See `src/forge/models.rs`.

## `include=versions` — abbreviated vs full

The `include=versions` query parameter behaves differently on the list vs single-mod endpoints:

| Endpoint | Versions returned | Fields included |
|----------|-------------------|-----------------|
| `GET /mods?include=versions` (list) | Last 6 | `id`, `version`, `spt_version`, `downloads` — **no** `link`, `content_length`, `fika_compatibility` |
| `GET /mod/{id}?include=versions` (single) | Last 10 | All fields including `link`, `content_length`, `fika_compatibility` |
| `GET /mod/{id}/versions` (dedicated) | Paginated (up to `per_page`) | All fields, supports `filter[spt_version]` |

This means you cannot rely on `link` being present on versions from the list endpoint. Use the dedicated versions endpoint when you need download URLs.

## `spt_version` vs `spt_version_constraint`

The field name varies by context:

- Version objects from mod endpoints use `spt_version` (a plain version string like `"3.10.0"`)
- Dependency node versions use `spt_version_constraint` (a semver constraint like `"~4.0.0"`)

Both are deserialized into `ForgeVersion.spt_version` via `#[serde(alias = "spt_version_constraint")]`.

## Category `name` vs `title`

The category object uses `name` on some endpoints and `title` on others. Handled via `#[serde(alias = "title")]` on `ForgeCategory.name`.

## Authentication

Bearer token via `Authorization: Bearer <token>` header. Optional — unauthenticated requests work but may have lower rate limits.

The client strips the auth token for downloads from non-Forge hosts (GitHub, GitLab, etc.) to avoid leaking credentials. See `ForgeClient::is_forge_url()`.

## Rate limiting

The API returns `429 Too Many Requests` with a `Retry-After` header (seconds). Quartermaster retries up to 2 times with the indicated delay (capped at 60s). Server errors (5xx) are also retried up to 2 times with immediate retry.

## Response caching

Quartermaster caches GET responses in-memory (256 entries, 5-minute TTL). Malformed JSON responses (garbled 200s from CDN errors) are not cached to avoid poisoning. The `check_updates` endpoint bypasses the cache since it's always called for fresh data.

## Mod identifiers

Mods can be referenced by:

- Numeric Forge ID (e.g., `42`)
- Slug (e.g., `big-brain`)
- GUID (e.g., `com.example.big-brain`) — used in dependency resolution
- Name (e.g., `"Big Brain"`) — resolved via search

The dependency resolution endpoint (`GET /mods/dependencies`) accepts either numeric IDs or GUIDs as the identifier in `mods=<id>:<version>` pairs.
