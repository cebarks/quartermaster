# DB Unit Tests & Forge Client Mock Tests Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Add comprehensive test coverage for the DB user/request layers (#19) and the Forge HTTP client (#20) using wiremock.

**Architecture:** Extend existing `src/db/tests.rs` with net-new DB tests (most already exist — only gaps need filling). Add `wiremock` as a dev-dependency and create a `#[cfg(test)] mod tests` block in `src/forge/client.rs` with `#[tokio::test]` async tests that run against a local mock HTTP server.

**Tech Stack:** Rust standard test framework, wiremock 0.6, tokio (already a dependency), tempfile (already a dependency)

## Global Constraints

- All tests run with `cargo test` — no special configuration
- DB tests are synchronous, using in-memory SQLite via `test_db()`
- Forge tests are async using `#[tokio::test]`
- Follow existing test patterns (`.unwrap()` on errors, `assert_eq!`/`assert!` assertions, check timestamps with `.is_some()` not exact values)
- Run `cargo test` and `cargo clippy -- -D warnings` before each commit

---

### Task 1: Add wiremock dependency and net-new DB tests

**Files:**
- Modify: `Cargo.toml` (line 99-102, dev-dependencies section)
- Modify: `src/db/tests.rs` (append after line 885)

**Interfaces:**
- Consumes: `Database::open_in_memory()`, `test_db()`, `setup_user()`, `setup_admin()` helpers, all `Database` methods from `users.rs` and `requests.rs`
- Produces: New test functions; wiremock available as dev-dependency for Task 2

**Context — what's already covered:**

The existing `db/tests.rs` (885 lines, 47 tests) already covers most of the spec. After careful audit, only these gaps remain:

1. **`list_users_alphabetical`** — existing test (`insert_and_get_user`, line 212) only inserts 1 user, never verifies sort order
2. **`disable_user_with_multiple_admins`** — `set_user_disabled_last_admin_guard` (line 526) tests the guard, but no test confirms disable succeeds when a backup admin exists
3. **`list_requests_downvote_score`** — existing `list_mod_requests_with_votes` (line 798) only tests upvotes; vote_score is always positive. Need a test with mixed up/downvotes to verify `vote_score = upvotes - downvotes`
4. **`update_invite_user`** — the unconditional `update_invite_user()` method (users.rs:261) has zero test coverage
5. **`list_requests_current_user_no_vote`** — existing tests always have the querying user vote; the `LEFT JOIN` + nullable `current_user_vote` path for a non-voting user is untested

- [ ] **Step 1: Add wiremock to dev-dependencies**

In `Cargo.toml`, add `wiremock` to the `[dev-dependencies]` section:

```toml
[dev-dependencies]
assert_cmd = "2"
predicates = "3"
temp-env = "0.3"
wiremock = "0.6"
```

- [ ] **Step 2: Write net-new DB tests**

Append the following tests to the end of `src/db/tests.rs`:

```rust
#[test]
fn list_users_alphabetical_order() {
    let db = test_db();
    db.insert_user("charlie", "p3", Some("pw"), Role::Player).unwrap();
    db.insert_user("alice", "p1", Some("pw"), Role::Admin).unwrap();
    db.insert_user("bob", "p2", Some("pw"), Role::Moderator).unwrap();

    let users = db.list_users().unwrap();
    assert_eq!(users.len(), 3);
    assert_eq!(users[0].username, "alice");
    assert_eq!(users[1].username, "bob");
    assert_eq!(users[2].username, "charlie");
}

#[test]
fn disable_admin_allowed_with_backup_admin() {
    let db = test_db();
    let admin1 = db.insert_user("admin1", "p1", Some("pw"), Role::Admin).unwrap();
    db.insert_user("admin2", "p2", Some("pw"), Role::Admin).unwrap();

    let affected = db.set_user_disabled(admin1, true).unwrap();
    assert_eq!(affected, 1);
    let user = db.get_user_by_id(admin1).unwrap().unwrap();
    assert!(user.disabled);
}

#[test]
fn update_invite_user_unconditional() {
    let db = test_db();
    let admin_id = db.insert_user("admin", "p1", Some("pw"), Role::Admin).unwrap();
    db.create_invite("CODE-1", Some(admin_id), None).unwrap();

    // Use the invite (sets used_by to admin_id)
    let used = db.use_invite("CODE-1", admin_id).unwrap();
    assert_eq!(used, 1);

    // Now unconditionally update to a different user
    let new_user = db.insert_user("newbie", "p2", Some("pw"), Role::Player).unwrap();
    let updated = db.update_invite_user("CODE-1", new_user).unwrap();
    assert_eq!(updated, 1);

    let invite = db.get_invite("CODE-1").unwrap().unwrap();
    assert_eq!(invite.used_by, Some(new_user));
}

#[test]
fn list_requests_mixed_votes_score() {
    let db = test_db();
    let user1 = setup_user(&db);
    let user2 = setup_admin(&db);
    let user3 = db.insert_user("voter3", "aid3", Some("hash"), Role::Player).unwrap();

    let req_id = db
        .create_mod_request(user1, 100, "Mod A", None, None, "unknown", None)
        .unwrap();

    // 2 upvotes, 1 downvote → score = 1
    db.upsert_vote(req_id, user1, true, None).unwrap();
    db.upsert_vote(req_id, user2, true, Some("good mod")).unwrap();
    db.upsert_vote(req_id, user3, false, Some("not needed")).unwrap();

    let views = db.list_mod_requests(Some("pending"), user1).unwrap();
    assert_eq!(views.len(), 1);
    assert_eq!(views[0].upvote_count, 2);
    assert_eq!(views[0].downvote_count, 1);
    assert_eq!(views[0].vote_score, 1);
    assert_eq!(views[0].comment_count, 2);
}

#[test]
fn list_requests_current_user_no_vote() {
    let db = test_db();
    let requester = setup_user(&db);
    let viewer = db.insert_user("viewer", "aid-v", Some("hash"), Role::Player).unwrap();

    db.create_mod_request(requester, 100, "Mod", None, None, "unknown", None)
        .unwrap();

    let views = db.list_mod_requests(Some("pending"), viewer).unwrap();
    assert_eq!(views.len(), 1);
    assert_eq!(views[0].current_user_vote, None);
}
```

- [ ] **Step 3: Run tests to verify they pass**

Run: `cargo test -p spt-quartermaster -- list_users_alphabetical_order disable_admin_allowed_with_backup_admin update_invite_user_unconditional list_requests_mixed_votes_score list_requests_current_user_no_vote`

Expected: All 5 tests pass.

- [ ] **Step 4: Run full test suite and clippy**

Run: `cargo test && cargo clippy -- -D warnings`

Expected: All 248+ tests pass, no clippy warnings.

- [ ] **Step 5: Commit**

```bash
git add Cargo.toml src/db/tests.rs
git commit -m "test: add wiremock dep and net-new DB unit tests (#19)

Add wiremock to dev-dependencies for upcoming Forge client tests.
Add 5 net-new DB tests covering gaps in existing coverage:
- list_users alphabetical ordering
- admin disable with backup admin
- unconditional invite user update
- mixed upvote/downvote score calculation
- current_user_vote is None when querying user has not voted"
```

---

### Task 2: Forge client happy path tests

**Files:**
- Modify: `src/forge/client.rs` (append `#[cfg(test)] mod tests` block after line 212)

**Interfaces:**
- Consumes: `ForgeClient::with_base_url()`, all public `ForgeClient` methods, all types from `forge::models`
- Produces: Test infrastructure (`test_client` helper) and 8 happy-path + request-validation tests

**Important:** These are the first `#[tokio::test]` tests in the project. The `tokio` crate is already a full dependency so `#[tokio::test]` works without any configuration change.

- [ ] **Step 1: Write test module scaffold and `search_mods` test**

Add the following at the end of `src/forge/client.rs`:

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use wiremock::matchers::{method, path, query_param};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    async fn test_client(server: &MockServer) -> ForgeClient {
        ForgeClient::with_base_url(server.uri(), None).unwrap()
    }

    #[tokio::test]
    async fn search_mods_returns_results() {
        let server = MockServer::start().await;
        let body = serde_json::json!({
            "data": [
                {
                    "id": 42,
                    "name": "Big Brain",
                    "slug": "big-brain",
                    "description": "AI overhaul",
                    "fika_compatibility": true
                },
                {
                    "id": 99,
                    "name": "SAIN",
                    "slug": "sain",
                    "fika_compatibility": false
                }
            ]
        });

        Mock::given(method("GET"))
            .and(path("/mods"))
            .and(query_param("query", "brain"))
            .respond_with(ResponseTemplate::new(200).set_body_json(&body))
            .expect(1)
            .mount(&server)
            .await;

        let client = test_client(&server).await;
        let mods = client.search_mods("brain").await.unwrap();

        assert_eq!(mods.len(), 2);
        assert_eq!(mods[0].id, 42);
        assert_eq!(mods[0].name, "Big Brain");
        assert_eq!(mods[0].slug.as_deref(), Some("big-brain"));
        assert_eq!(mods[0].fika_compatibility, Some(FikaCompat::Compatible));
        assert_eq!(mods[1].id, 99);
        assert_eq!(mods[1].fika_compatibility, Some(FikaCompat::Incompatible));
    }

    #[tokio::test]
    async fn get_mod_without_versions() {
        let server = MockServer::start().await;
        let body = serde_json::json!({
            "data": {
                "id": 42,
                "name": "Big Brain",
                "slug": "big-brain",
                "description": "AI overhaul",
                "fika_compatibility": true
            }
        });

        Mock::given(method("GET"))
            .and(path("/mod/42"))
            .respond_with(ResponseTemplate::new(200).set_body_json(&body))
            .expect(1)
            .mount(&server)
            .await;

        let client = test_client(&server).await;
        let m = client.get_mod(42, false).await.unwrap();

        assert_eq!(m.id, 42);
        assert_eq!(m.name, "Big Brain");
        assert!(m.versions.is_none());
    }

    #[tokio::test]
    async fn get_mod_with_versions() {
        let server = MockServer::start().await;
        let body = serde_json::json!({
            "data": {
                "id": 42,
                "name": "Big Brain",
                "fika_compatibility": true,
                "versions": [
                    {
                        "id": 100,
                        "version": "1.2.0",
                        "spt_version": "3.9.0",
                        "link": "https://example.com/download",
                        "content_length": 1048576,
                        "fika_compatibility": "compatible",
                        "dependencies": []
                    },
                    {
                        "id": 101,
                        "version": "1.1.0",
                        "spt_version": "3.8.0"
                    }
                ]
            }
        });

        Mock::given(method("GET"))
            .and(path("/mod/42"))
            .and(query_param("include", "versions"))
            .respond_with(ResponseTemplate::new(200).set_body_json(&body))
            .expect(1)
            .mount(&server)
            .await;

        let client = test_client(&server).await;
        let m = client.get_mod(42, true).await.unwrap();

        let versions = m.versions.expect("should have versions");
        assert_eq!(versions.len(), 2);
        assert_eq!(versions[0].id, 100);
        assert_eq!(versions[0].version, "1.2.0");
        assert_eq!(versions[0].link.as_deref(), Some("https://example.com/download"));
        assert_eq!(versions[1].id, 101);
        assert!(versions[1].link.is_none());
    }

    #[tokio::test]
    async fn get_versions_with_spt_filter() {
        let server = MockServer::start().await;
        let body = serde_json::json!({
            "data": [
                {
                    "id": 100,
                    "version": "1.2.0",
                    "spt_version": "3.10.0",
                    "fika_compatibility": "compatible"
                }
            ]
        });

        Mock::given(method("GET"))
            .and(path("/mod/42/versions"))
            .and(query_param("filter[spt_version]", "3.10.0"))
            .respond_with(ResponseTemplate::new(200).set_body_json(&body))
            .expect(1)
            .mount(&server)
            .await;

        let client = test_client(&server).await;
        let versions = client.get_versions(42, Some("3.10.0")).await.unwrap();

        assert_eq!(versions.len(), 1);
        assert_eq!(versions[0].spt_version.as_deref(), Some("3.10.0"));
    }

    #[tokio::test]
    async fn get_versions_no_spt_filter_omits_param() {
        let server = MockServer::start().await;
        let body = serde_json::json!({
            "data": [
                {"id": 100, "version": "1.2.0"},
                {"id": 101, "version": "1.1.0"}
            ]
        });

        Mock::given(method("GET"))
            .and(path("/mod/42/versions"))
            .respond_with(ResponseTemplate::new(200).set_body_json(&body))
            .expect(1)
            .mount(&server)
            .await;

        let client = test_client(&server).await;
        let versions = client.get_versions(42, None).await.unwrap();
        assert_eq!(versions.len(), 2);
    }

    #[tokio::test]
    async fn check_updates_parses_response() {
        let server = MockServer::start().await;
        let body = serde_json::json!({
            "data": {
                "spt_version": "3.10.0",
                "updates": [
                    {
                        "current_version": {
                            "id": 100,
                            "mod_id": 42,
                            "name": "Big Brain",
                            "slug": "big-brain",
                            "version": "1.1.0"
                        },
                        "recommended_version": {
                            "id": 200,
                            "version": "1.2.0",
                            "link": "https://example.com/dl",
                            "content_length": 2048,
                            "fika_compatibility": "compatible"
                        },
                        "update_reason": "newer version available"
                    }
                ],
                "blocked_updates": [],
                "up_to_date": [],
                "incompatible_with_spt": []
            }
        });

        Mock::given(method("GET"))
            .and(path("/mods/updates"))
            .respond_with(ResponseTemplate::new(200).set_body_json(&body))
            .expect(1)
            .mount(&server)
            .await;

        let client = test_client(&server).await;
        let result = client
            .check_updates(&[(42, "1.1.0".to_string())], "3.10.0")
            .await
            .unwrap();

        assert_eq!(result.spt_version, "3.10.0");
        assert_eq!(result.updates.len(), 1);
        assert_eq!(result.updates[0].current_version.name, "Big Brain");
        assert_eq!(result.updates[0].recommended_version.version, "1.2.0");
        assert_eq!(result.updates[0].update_reason, "newer version available");
        assert!(result.blocked_updates.is_empty());
        assert!(result.incompatible_with_spt.is_empty());
    }

    #[tokio::test]
    async fn get_dependencies_parses_tree() {
        let server = MockServer::start().await;
        let body = serde_json::json!({
            "data": [
                {
                    "id": 42,
                    "name": "Big Brain",
                    "slug": "big-brain",
                    "latest_compatible_version": {
                        "id": 100,
                        "version": "1.2.0",
                        "spt_version_constraint": "~3.10.0",
                        "link": "https://example.com/dl",
                        "content_length": 2048,
                        "fika_compatibility": "compatible"
                    },
                    "dependencies": [
                        {
                            "id": 10,
                            "name": "CoreLib",
                            "slug": "corelib",
                            "latest_compatible_version": {
                                "id": 50,
                                "version": "0.5.0"
                            },
                            "dependencies": [],
                            "conflict": false
                        }
                    ],
                    "conflict": false
                }
            ]
        });

        Mock::given(method("GET"))
            .and(path("/mods/dependencies"))
            .and(query_param("mods", "42:1.2.0"))
            .respond_with(ResponseTemplate::new(200).set_body_json(&body))
            .expect(1)
            .mount(&server)
            .await;

        let client = test_client(&server).await;
        let deps = client.get_dependencies(&[(42, "1.2.0")]).await.unwrap();

        assert_eq!(deps.len(), 1);
        assert_eq!(deps[0].id, 42);
        assert_eq!(deps[0].name, "Big Brain");
        assert!(!deps[0].conflict);

        let version = deps[0].latest_compatible_version.as_ref().unwrap();
        assert_eq!(version.version, "1.2.0");
        assert_eq!(version.spt_version.as_deref(), Some("~3.10.0"));

        assert_eq!(deps[0].dependencies.len(), 1);
        assert_eq!(deps[0].dependencies[0].name, "CoreLib");
        assert!(!deps[0].dependencies[0].conflict);
    }

    #[tokio::test]
    async fn check_updates_formats_multiple_mods() {
        let server = MockServer::start().await;
        let body = serde_json::json!({
            "data": {
                "spt_version": "3.10.0",
                "updates": [],
                "blocked_updates": [],
                "up_to_date": [],
                "incompatible_with_spt": []
            }
        });

        Mock::given(method("GET"))
            .and(path("/mods/updates"))
            .and(query_param("mods", "42:1.0.0,99:2.0.0"))
            .and(query_param("spt_version", "3.10.0"))
            .respond_with(ResponseTemplate::new(200).set_body_json(&body))
            .expect(1)
            .mount(&server)
            .await;

        let client = test_client(&server).await;
        let result = client
            .check_updates(
                &[(42, "1.0.0".into()), (99, "2.0.0".into())],
                "3.10.0",
            )
            .await
            .unwrap();

        assert_eq!(result.spt_version, "3.10.0");
        assert!(result.updates.is_empty());
    }
}
```

- [ ] **Step 2: Run tests to verify they pass**

Run: `cargo test -p spt-quartermaster forge::client::tests`

Expected: All 9 tests pass (7 happy path + 2 request validation).

- [ ] **Step 3: Run clippy**

Run: `cargo clippy -- -D warnings`

Expected: No warnings.

- [ ] **Step 4: Commit**

```bash
git add src/forge/client.rs
git commit -m "test: add Forge client happy path tests with wiremock (#20)

Add wiremock-backed tests for all ForgeClient public methods:
search_mods, get_mod (with/without versions), get_versions
(with/without SPT filter), check_updates, and get_dependencies.
Each test spins up a local mock HTTP server, verifies the correct
endpoint is called, and asserts the response is parsed correctly."
```

---

### Task 3: Forge client error handling and download tests

**Files:**
- Modify: `src/forge/client.rs` (append to `mod tests` block)

**Interfaces:**
- Consumes: `test_client()` helper from Task 2, `ForgeClient::search_mods()`, `ForgeClient::get_mod()`, `ForgeClient::download_file()`
- Produces: 6 error-handling tests

- [ ] **Step 1: Write error handling and download tests**

Append the following tests inside the existing `mod tests` block in `src/forge/client.rs`:

```rust
    #[tokio::test]
    async fn search_mods_404_returns_error() {
        let server = MockServer::start().await;

        Mock::given(method("GET"))
            .and(path("/mods"))
            .respond_with(ResponseTemplate::new(404))
            .expect(1)
            .mount(&server)
            .await;

        let client = test_client(&server).await;
        let result = client.search_mods("anything").await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn search_mods_500_returns_error() {
        let server = MockServer::start().await;

        Mock::given(method("GET"))
            .and(path("/mods"))
            .respond_with(ResponseTemplate::new(500))
            .expect(1)
            .mount(&server)
            .await;

        let client = test_client(&server).await;
        let result = client.search_mods("anything").await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn get_mod_not_found_returns_error() {
        let server = MockServer::start().await;

        Mock::given(method("GET"))
            .and(path("/mod/99999"))
            .respond_with(ResponseTemplate::new(404))
            .expect(1)
            .mount(&server)
            .await;

        let client = test_client(&server).await;
        let result = client.get_mod(99999, false).await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn malformed_json_returns_error() {
        let server = MockServer::start().await;

        Mock::given(method("GET"))
            .and(path("/mods"))
            .respond_with(
                ResponseTemplate::new(200)
                    .set_body_string("this is not json"),
            )
            .expect(1)
            .mount(&server)
            .await;

        let client = test_client(&server).await;
        let result = client.search_mods("test").await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn download_file_writes_to_disk() {
        let server = MockServer::start().await;
        let file_content = b"fake archive content for testing";

        Mock::given(method("GET"))
            .and(path("/files/test.zip"))
            .respond_with(
                ResponseTemplate::new(200)
                    .set_body_bytes(file_content.to_vec()),
            )
            .expect(1)
            .mount(&server)
            .await;

        let client = test_client(&server).await;
        let dir = tempfile::tempdir().unwrap();
        let dest = dir.path().join("downloaded.zip");

        let url = format!("{}/files/test.zip", server.uri());
        client.download_file(&url, &dest).await.unwrap();

        let written = std::fs::read(&dest).unwrap();
        assert_eq!(written, file_content);
    }

    #[tokio::test]
    async fn download_file_404_returns_error() {
        let server = MockServer::start().await;

        Mock::given(method("GET"))
            .and(path("/files/missing.zip"))
            .respond_with(ResponseTemplate::new(404))
            .expect(1)
            .mount(&server)
            .await;

        let client = test_client(&server).await;
        let dir = tempfile::tempdir().unwrap();
        let dest = dir.path().join("missing.zip");

        let url = format!("{}/files/missing.zip", server.uri());
        let result = client.download_file(&url, &dest).await;
        assert!(result.is_err());
    }
```

- [ ] **Step 2: Run tests to verify they pass**

Run: `cargo test -p spt-quartermaster forge::client::tests`

Expected: All 15 tests pass (9 from Task 2 + 6 new).

- [ ] **Step 3: Run clippy**

Run: `cargo clippy -- -D warnings`

Expected: No warnings.

- [ ] **Step 4: Commit**

```bash
git add src/forge/client.rs
git commit -m "test: add Forge client error handling and download tests (#20)

Test HTTP error propagation (404, 500), malformed JSON handling,
and file download to disk. All use wiremock mock server."
```

---

### Task 4: Forge client API quirk and auth tests

**Files:**
- Modify: `src/forge/client.rs` (append to `mod tests` block)

**Interfaces:**
- Consumes: `ForgeClient::with_base_url()` (with token), `test_client()`, `ForgeClient::search_mods()`, `ForgeClient::get_mod()`, `ForgeClient::check_updates()`
- Produces: 4 quirk/validation tests

- [ ] **Step 1: Write API quirk and auth tests**

Append the following tests inside the existing `mod tests` block in `src/forge/client.rs`:

```rust
    #[tokio::test]
    async fn fika_compat_bool_on_mod_object() {
        let server = MockServer::start().await;
        let body = serde_json::json!({
            "data": {
                "id": 42,
                "name": "Test Mod",
                "fika_compatibility": true
            }
        });

        Mock::given(method("GET"))
            .and(path("/mod/42"))
            .respond_with(ResponseTemplate::new(200).set_body_json(&body))
            .mount(&server)
            .await;

        let client = test_client(&server).await;
        let m = client.get_mod(42, false).await.unwrap();
        assert_eq!(m.fika_compatibility, Some(FikaCompat::Compatible));
    }

    #[tokio::test]
    async fn fika_compat_string_on_version_object() {
        let server = MockServer::start().await;
        let body = serde_json::json!({
            "data": {
                "id": 42,
                "name": "Test Mod",
                "fika_compatibility": true,
                "versions": [
                    {
                        "id": 100,
                        "version": "1.0.0",
                        "fika_compatibility": "incompatible"
                    }
                ]
            }
        });

        Mock::given(method("GET"))
            .and(path("/mod/42"))
            .and(query_param("include", "versions"))
            .respond_with(ResponseTemplate::new(200).set_body_json(&body))
            .mount(&server)
            .await;

        let client = test_client(&server).await;
        let m = client.get_mod(42, true).await.unwrap();

        assert_eq!(m.fika_compatibility, Some(FikaCompat::Compatible));
        let v = &m.versions.unwrap()[0];
        assert_eq!(v.fika_compatibility, Some(FikaCompat::Incompatible));
    }

    #[tokio::test]
    async fn abbreviated_versions_missing_optional_fields() {
        let server = MockServer::start().await;
        let body = serde_json::json!({
            "data": {
                "id": 42,
                "name": "Test Mod",
                "fika_compatibility": true,
                "versions": [
                    {
                        "id": 100,
                        "version": "1.0.0"
                    }
                ]
            }
        });

        Mock::given(method("GET"))
            .and(path("/mod/42"))
            .and(query_param("include", "versions"))
            .respond_with(ResponseTemplate::new(200).set_body_json(&body))
            .mount(&server)
            .await;

        let client = test_client(&server).await;
        let m = client.get_mod(42, true).await.unwrap();
        let v = &m.versions.unwrap()[0];

        assert!(v.link.is_none());
        assert!(v.content_length.is_none());
        assert!(v.fika_compatibility.is_none());
        assert!(v.spt_version.is_none());
        assert!(v.dependencies.is_none());
    }

    #[tokio::test]
    async fn auth_token_sent_in_header() {
        let server = MockServer::start().await;
        let body = serde_json::json!({"data": []});

        Mock::given(method("GET"))
            .and(path("/mods"))
            .and(wiremock::matchers::header("Authorization", "Bearer test-token-123"))
            .respond_with(ResponseTemplate::new(200).set_body_json(&body))
            .expect(1)
            .mount(&server)
            .await;

        let client = ForgeClient::with_base_url(
            server.uri(),
            Some("test-token-123".to_string()),
        )
        .unwrap();

        let mods = client.search_mods("test").await.unwrap();
        assert!(mods.is_empty());
    }
```

- [ ] **Step 2: Run all Forge tests**

Run: `cargo test -p spt-quartermaster forge::client::tests`

Expected: All 19 tests pass (15 from previous tasks + 4 new).

- [ ] **Step 3: Run full test suite and clippy**

Run: `cargo test && cargo clippy -- -D warnings`

Expected: All tests pass (257+ total), no clippy warnings.

- [ ] **Step 4: Commit**

```bash
git add src/forge/client.rs
git commit -m "test: add Forge API quirk and auth token tests (#20)

Test the fika_compatibility dual representation (bool on mod objects,
string enum on version objects), abbreviated versions with missing
optional fields, and Bearer token header injection."
```
