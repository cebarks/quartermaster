# DB Unit Tests & Forge Client Mock Tests Design

**Issues**: #19 (Add DB request and user layer unit tests), #20 (Add Forge client tests with mock HTTP)
**Date**: 2026-06-19

## Overview

Add comprehensive test coverage for two foundational layers: the database user/request/voting operations and the Forge API HTTP client. These are the two least-tested areas that other features depend on, making them high-value targets.

## Approach

- **DB tests**: Extend existing `src/db/tests.rs` with new test sections. Reuse `test_db()`, `setup_user()`, `setup_admin()` helpers.
- **Forge tests**: Add `wiremock` dev-dependency. Create `#[cfg(test)] mod tests` in `src/forge/client.rs` using `#[tokio::test]` (first async tests in the project; `tokio` is already a full dependency). Point `ForgeClient::with_base_url()` at wiremock's `MockServer`.

## New Dependencies

```toml
[dev-dependencies]
wiremock = "0.6"
```

No changes to runtime dependencies.

## #19 — DB User & Request Layer Unit Tests

### Location

`src/db/tests.rs` — extend the existing test module.

### User Layer Tests (~12 tests)

| Test | What it verifies |
|------|-----------------|
| `insert_and_get_user_by_username` | Insert user, retrieve by username, verify all 8 fields |
| `get_user_by_id` | Retrieve by numeric id, verify match |
| `get_nonexistent_user_returns_none` | Returns None for missing user |
| `list_users_alphabetical` | Multiple users sorted by username |
| `admin_exists_false_on_empty_db` | No users → false |
| `admin_exists_false_with_players_only` | Players only → false |
| `count_admins_excludes_disabled` | Disabled admins not counted |
| `update_user_password_sets_timestamp` | Hash changes, `password_changed_at` set |
| `update_user_role_last_admin_blocked` | Sole admin demotion returns 0 |
| `update_user_role_multiple_admins_allowed` | Demotion succeeds with backup admin |
| `disable_user_last_admin_blocked` | Sole admin disable returns 0 |
| `disable_user_multiple_admins_allowed` | Disable succeeds with backup admin |

**Note**: Some of these may overlap with existing tests. During implementation, skip any that are already covered and add only net-new coverage.

### Invite & Reset Token Tests (~8 tests)

| Test | What it verifies |
|------|-----------------|
| `create_and_get_invite_roundtrip` | Insert invite, retrieve by code, all fields match |
| `use_invite_marks_used` | Sets used_by/used_at, subsequent use returns 0 |
| `expired_invite_rejected` | Past expiry date → use returns 0 |
| `list_invite_codes_joins_usernames` | JOIN populates creator and user usernames |
| `create_reset_token_replaces_old` | Second token for same user deletes first |
| `get_reset_token_roundtrip` | Insert and retrieve token |
| `delete_reset_token` | Token removed after delete |

### Pending Operations Tests (~5 tests)

| Test | What it verifies |
|------|-----------------|
| `insert_and_list_pending_ops` | Round-trip, ordered by queued_at |
| `pending_op_with_queued_by` | queued_by field populated correctly |
| `delete_single_pending_op` | Remove one, others remain |
| `clear_all_pending_ops` | Bulk delete empties table |

### Request & Voting Tests (~10 tests)

| Test | What it verifies |
|------|-----------------|
| `create_and_get_mod_request` | Round-trip, all 14 fields |
| `has_pending_request_for_mod` | True when pending, false when resolved/absent |
| `resolve_mod_request_approve` | Sets resolved_by/resolved_at/status |
| `resolve_already_resolved_returns_zero` | No double-resolve |
| `update_mod_request_cache` | Metadata + forge_cached_at updated |
| `list_requests_filter_by_status` | Filter pending/approved/rejected separately |
| `list_requests_vote_aggregation` | vote_score, up/down counts, comment_count |
| `upsert_vote_toggle` | Upvote → downvote for same user |
| `delete_vote_updates_score` | Remove vote, score decreases |
| `list_vote_comments_excludes_empty` | Only returns votes with non-empty comment text |

## #20 — Forge Client Tests with Mock HTTP

### Location

New `#[cfg(test)] mod tests` block in `src/forge/client.rs`.

### Test Helper

```rust
async fn test_client(server: &MockServer) -> ForgeClient {
    ForgeClient::with_base_url(server.uri(), None).unwrap()
}
```

### Happy Path Tests (~6 tests)

| Test | Endpoint mocked | What it verifies |
|------|----------------|-----------------|
| `search_mods_returns_results` | `GET /mods?query=fika` | Parsed `Vec<ForgeMod>` with correct fields |
| `get_mod_without_versions` | `GET /mod/42` | Single `ForgeMod`, versions is None |
| `get_mod_with_versions` | `GET /mod/42?include=versions` | Versions vec populated |
| `get_versions_with_spt_filter` | `GET /mod/42/versions?filter[spt_version]=3.10.0` | Filter query param sent, versions parsed |
| `check_updates` | `GET /mods/updates?...` | `UpdatesResponseData` fully parsed |
| `get_dependencies` | `GET /mods/dependencies?mods=42:1.0.0` | Recursive `DependencyNode` tree parsed |

### Error Handling Tests (~5 tests)

| Test | Scenario | What it verifies |
|------|---------|-----------------|
| `search_mods_404` | Server returns 404 | Error propagated (not panic) |
| `search_mods_500` | Server returns 500 | Error propagated |
| `get_mod_not_found` | 404 for specific mod | Error with context |
| `malformed_json_response` | Invalid JSON body | Parse error propagated |
| `download_file_writes_to_disk` | Mock file download | File written correctly to temp path |

### Forge API Quirk Tests (~5 tests)

| Test | Quirk | What it verifies |
|------|-------|-----------------|
| `fika_compat_bool_on_mod` | `fika_compatibility: true` on mod object | Deserializes to `FikaCompat::Compatible` |
| `fika_compat_string_on_version` | `fika_compatibility: "incompatible"` on version | Deserializes to `FikaCompat::Incompatible` |
| `abbreviated_versions_missing_fields` | Versions without `link`/`content_length`/`fika_compatibility` | All parse as None |
| `full_versions_all_fields` | All optional fields present | Parsed correctly |
| `auth_token_sent_in_header` | Client created with token | Authorization Bearer header present in request |

### Request Validation Tests (~2 tests)

| Test | What it verifies |
|------|-----------------|
| `get_versions_no_spt_filter_omits_param` | No `filter` query param when spt_version is None |
| `check_updates_formats_mod_list` | `mods=id1:ver1,id2:ver2` format correct |

## Test Execution

All tests run with `cargo test`. No special configuration needed.

- DB tests: synchronous, in-memory SQLite — fast, no cleanup
- Forge tests: async (`#[tokio::test]`), wiremock spins up a local server per test — isolated, parallel-safe

## Out of Scope

- Web handler integration tests (#7) — separate issue, depends on patterns established here
- CLI command integration tests (#18) — separate issue
- Code coverage reporting — not adding CI coverage tools in this pass
- Property-based testing — standard example-based tests are sufficient for these layers
