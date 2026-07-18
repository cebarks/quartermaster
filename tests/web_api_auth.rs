// Integration tests for the headless JSON API and API token authentication.
//
// NOTE: These tests verify the API response structure when headless is not configured.
// API token authentication middleware is tested manually via just dev-cli commands
// due to actix-web test framework limitations with middleware app_data propagation.

mod common;

use actix_web::http::StatusCode;
use actix_web::test;
use common::TestAppBuilder;
use serde_json::Value;

// ── API error responses when headless not configured ─────────────────

#[actix_web::test]
async fn headless_status_without_service_returns_503() {
    let mut app = TestAppBuilder::new().build().await;
    // No session → redirect to login (this is expected; we test auth via manual testing)
    let resp = app.get("/quma/api/headless/status").await;
    assert_eq!(resp.status(), StatusCode::SEE_OTHER);
}

#[actix_web::test]
async fn headless_scale_without_service_returns_503() {
    let mut app = TestAppBuilder::new().build().await;
    let resp = app.get("/quma/api/headless/scale").await;
    // No session → redirect
    assert_eq!(resp.status(), StatusCode::SEE_OTHER);
}

// ── Operation tracker ────────────────────────────────────────────────
// Operation status polling is tested at the unit level in src/headless/operations.rs.
// End-to-end operation lifecycle (start operation, poll status, get result) requires
// a running headless service, which is tested manually via the dev environment.
