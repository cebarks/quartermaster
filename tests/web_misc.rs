mod common;

use actix_web::http::StatusCode;
use common::TestAppBuilder;

// Static assets must be served without authentication so the login page can load CSS/JS
#[actix_web::test]
async fn assets_served_without_auth() {
    let mut app = TestAppBuilder::new().build().await;

    let resp = app.get("/quma/assets/style.css").await;
    assert_eq!(
        resp.status(),
        StatusCode::OK,
        "static assets must not require auth"
    );
}

// Browsers reject stylesheets/scripts without the correct Content-Type header
#[actix_web::test]
async fn assets_have_correct_content_type() {
    let mut app = TestAppBuilder::new().build().await;

    let resp = app.get("/quma/assets/style.css").await;
    let ct = resp
        .headers()
        .get("content-type")
        .expect("missing Content-Type on CSS");
    assert_eq!(ct, "text/css; charset=utf-8");

    let resp = app.get("/quma/assets/htmx.min.js").await;
    let ct = resp
        .headers()
        .get("content-type")
        .expect("missing Content-Type on JS");
    assert_eq!(ct, "application/javascript; charset=utf-8");
}

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
        .with_user("admin", "pass", "admin")
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
        .with_user("player", "pass", "player")
        .build()
        .await;

    app.login_as("player", "pass").await;

    let resp = app.get("/quma/metrics").await;
    assert_eq!(resp.status(), StatusCode::FORBIDDEN);
}

#[actix_web::test]
async fn metrics_loads_for_admin() {
    let mut app = TestAppBuilder::new()
        .with_user("admin", "pass", "admin")
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
        .with_user("admin", "pass", "admin")
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
        .with_user("admin", "pass", "admin")
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
        .with_user("admin", "pass", "admin")
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

// Convoy tests
#[actix_web::test]
async fn convoy_page_requires_auth() {
    let mut app = TestAppBuilder::new().build().await;

    let resp = app.get("/quma/convoy").await;
    assert_eq!(resp.status(), StatusCode::SEE_OTHER);
}

#[actix_web::test]
async fn convoy_page_loads() {
    let mut app = TestAppBuilder::new()
        .with_user("admin", "pass", "admin")
        .build()
        .await;

    app.login_as("admin", "pass").await;

    let resp = app.get("/quma/convoy").await;
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
        .with_user("admin", "pass", "admin")
        .build()
        .await;

    app.login_as("admin", "pass").await;

    let resp = app.get("/quma/api/mods/requests").await;
    assert_eq!(resp.status(), StatusCode::OK);
}

// Groups tab tests
#[actix_web::test]
async fn mods_groups_tab_requires_auth() {
    let mut app = TestAppBuilder::new().build().await;

    let resp = app.get("/quma/api/mods/groups").await;
    assert_eq!(resp.status(), StatusCode::SEE_OTHER);
}

#[actix_web::test]
async fn mods_groups_tab_loads() {
    let mut app = TestAppBuilder::new()
        .with_user("admin", "pass", "admin")
        .build()
        .await;

    app.login_as("admin", "pass").await;

    let resp = app.get("/quma/api/mods/groups").await;
    assert_eq!(resp.status(), StatusCode::OK);
}
