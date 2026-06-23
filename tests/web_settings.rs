mod common;

use actix_web::http::StatusCode;
use common::TestAppBuilder;

#[actix_web::test]
async fn settings_page_requires_admin() {
    let mut app = TestAppBuilder::new()
        .with_user("player", "password", "player")
        .build()
        .await;

    app.login_as("player", "password").await;

    let response = app.get("/quma/settings").await;

    assert_eq!(response.status(), StatusCode::FORBIDDEN);
}

#[actix_web::test]
async fn settings_page_accessible_by_admin() {
    let mut app = TestAppBuilder::new()
        .with_user("admin", "password", "admin")
        .build()
        .await;

    app.login_as("admin", "password").await;

    let response = app.get("/quma/settings").await;

    assert_eq!(response.status(), StatusCode::OK);
}

#[actix_web::test]
async fn settings_moderator_gets_403() {
    let mut app = TestAppBuilder::new()
        .with_user("moderator", "password", "moderator")
        .build()
        .await;

    app.login_as("moderator", "password").await;

    let response = app.get("/quma/settings").await;

    assert_eq!(response.status(), StatusCode::FORBIDDEN);
}
