mod common;

use actix_web::http::StatusCode;
use common::TestAppBuilder;
use spt_quartermaster::db::users::Role;

// Logs tests
#[actix_web::test]
async fn logs_page_requires_auth() {
    let mut app = TestAppBuilder::new().build().await;

    let resp = app.get("/quma/logs").await;
    assert_eq!(resp.status(), StatusCode::SEE_OTHER);
}

#[actix_web::test]
async fn logs_page_loads() {
    let mut app = TestAppBuilder::new()
        .with_user("admin", "pass", Role::Admin)
        .build()
        .await;

    app.login_as("admin", "pass").await;

    let resp = app.get("/quma/logs").await;
    assert_eq!(resp.status(), StatusCode::OK);
}

#[actix_web::test]
async fn app_logs_json_requires_auth() {
    let mut app = TestAppBuilder::new().build().await;

    let resp = app.get("/quma/api/logs/app").await;
    assert_eq!(resp.status(), StatusCode::SEE_OTHER);
}

// Metrics tests
#[actix_web::test]
async fn metrics_page_requires_auth() {
    let mut app = TestAppBuilder::new().build().await;

    let resp = app.get("/quma/metrics").await;
    assert_eq!(resp.status(), StatusCode::SEE_OTHER);
}

#[actix_web::test]
async fn metrics_requires_server_control() {
    let mut app = TestAppBuilder::new()
        .with_user("player", "pass", Role::Player)
        .build()
        .await;

    app.login_as("player", "pass").await;

    let resp = app.get("/quma/metrics").await;
    assert_eq!(resp.status(), StatusCode::FORBIDDEN);
}

#[actix_web::test]
async fn metrics_loads_for_admin() {
    let mut app = TestAppBuilder::new()
        .with_user("admin", "pass", Role::Admin)
        .build()
        .await;

    app.login_as("admin", "pass").await;

    let resp = app.get("/quma/metrics").await;
    assert_eq!(resp.status(), StatusCode::OK);
}

// Tasks tests
#[actix_web::test]
async fn task_status_requires_auth() {
    let mut app = TestAppBuilder::new().build().await;

    let resp = app.get("/quma/api/tasks/status").await;
    assert_eq!(resp.status(), StatusCode::SEE_OTHER);
}

#[actix_web::test]
async fn task_status_loads() {
    let mut app = TestAppBuilder::new()
        .with_user("admin", "pass", Role::Admin)
        .build()
        .await;

    app.login_as("admin", "pass").await;

    let resp = app.get("/quma/api/tasks/status").await;
    assert_eq!(resp.status(), StatusCode::OK);
}

// Profiles tests
#[actix_web::test]
async fn profile_page_requires_auth() {
    let mut app = TestAppBuilder::new().build().await;

    let resp = app.get("/quma/profiles/someone").await;
    assert_eq!(resp.status(), StatusCode::SEE_OTHER);
}

#[actix_web::test]
async fn profile_partials_require_auth() {
    let mut app = TestAppBuilder::new().build().await;

    let resp = app.get("/quma/api/profiles/someone/quests").await;
    assert_eq!(resp.status(), StatusCode::SEE_OTHER);
}

// Raids tests
#[actix_web::test]
async fn stats_page_requires_auth() {
    let mut app = TestAppBuilder::new().build().await;

    let resp = app.get("/quma/stats").await;
    assert_eq!(resp.status(), StatusCode::SEE_OTHER);
}

#[actix_web::test]
async fn stats_page_loads() {
    let mut app = TestAppBuilder::new()
        .with_user("admin", "pass", Role::Admin)
        .build()
        .await;

    app.login_as("admin", "pass").await;

    let resp = app.get("/quma/stats").await;
    assert_eq!(resp.status(), StatusCode::OK);
}

#[actix_web::test]
async fn raids_partial_requires_auth() {
    let mut app = TestAppBuilder::new().build().await;

    let resp = app.get("/quma/api/raids/recent").await;
    assert_eq!(resp.status(), StatusCode::SEE_OTHER);
}

// SVM tests
#[actix_web::test]
async fn svm_page_requires_auth() {
    let mut app = TestAppBuilder::new().build().await;

    let resp = app.get("/quma/svm").await;
    assert_eq!(resp.status(), StatusCode::SEE_OTHER);
}

#[actix_web::test]
async fn svm_page_loads() {
    let mut app = TestAppBuilder::new()
        .with_user("admin", "pass", Role::Admin)
        .build()
        .await;

    app.login_as("admin", "pass").await;

    let resp = app.get("/quma/svm").await;
    // SVM not installed in test env (svm: None), so handler returns 404
    assert!(
        resp.status() == StatusCode::OK || resp.status() == StatusCode::NOT_FOUND,
        "unexpected status: {}",
        resp.status()
    );
}

// Modsync tests
#[actix_web::test]
async fn modsync_page_requires_auth() {
    let mut app = TestAppBuilder::new().build().await;

    let resp = app.get("/quma/modsync").await;
    assert_eq!(resp.status(), StatusCode::SEE_OTHER);
}

#[actix_web::test]
async fn modsync_page_loads() {
    let mut app = TestAppBuilder::new()
        .with_user("admin", "pass", Role::Admin)
        .build()
        .await;

    app.login_as("admin", "pass").await;

    let resp = app.get("/quma/modsync").await;
    assert_eq!(resp.status(), StatusCode::OK);
}

// Requests tests
#[actix_web::test]
async fn requests_tab_requires_auth() {
    let mut app = TestAppBuilder::new().build().await;

    let resp = app.get("/quma/api/mods/requests").await;
    assert_eq!(resp.status(), StatusCode::SEE_OTHER);
}

#[actix_web::test]
async fn requests_tab_loads() {
    let mut app = TestAppBuilder::new()
        .with_user("admin", "pass", Role::Admin)
        .build()
        .await;

    app.login_as("admin", "pass").await;

    let resp = app.get("/quma/api/mods/requests").await;
    assert_eq!(resp.status(), StatusCode::OK);
}
