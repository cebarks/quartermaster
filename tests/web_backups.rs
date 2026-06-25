mod common;

use actix_web::http::StatusCode;
use common::TestAppBuilder;

#[actix_web::test]
async fn mod_backups_partial_requires_auth() {
    let mut app = TestAppBuilder::new().build().await;
    let resp = app.get("/quma/mods/1/backups").await;
    assert_eq!(resp.status(), StatusCode::SEE_OTHER);
}

#[actix_web::test]
async fn admin_backups_requires_auth() {
    let mut app = TestAppBuilder::new().build().await;
    let resp = app.get("/quma/admin/backups").await;
    assert_eq!(resp.status(), StatusCode::SEE_OTHER);
}

#[actix_web::test]
async fn admin_backups_requires_settings_manage() {
    let mut app = TestAppBuilder::new()
        .with_user("player", "password123", "player")
        .build()
        .await;
    app.login_as("player", "password123").await;
    let resp = app.get("/quma/admin/backups").await;
    assert_eq!(resp.status(), StatusCode::FORBIDDEN);
}

#[actix_web::test]
async fn admin_backups_loads_for_admin() {
    let mut app = TestAppBuilder::new()
        .with_user("admin", "password123", "admin")
        .build()
        .await;
    app.login_as("admin", "password123").await;
    let resp = app.get("/quma/admin/backups").await;
    assert_eq!(resp.status(), StatusCode::OK);
}
