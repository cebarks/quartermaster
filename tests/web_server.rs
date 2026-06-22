#[path = "web_helpers.rs"]
mod web_helpers;

use actix_web::http::StatusCode;
use spt_quartermaster::db::users::Role;
use web_helpers::TestAppBuilder;

#[actix_web::test]
async fn server_start_requires_auth() {
    let mut app = TestAppBuilder::new().build().await;

    let resp = app
        .post_form("/quma/server/start", "csrf_token=dummy")
        .await;
    assert_eq!(resp.status(), StatusCode::SEE_OTHER);
}

#[actix_web::test]
async fn server_start_requires_capability() {
    let mut app = TestAppBuilder::new()
        .with_user("player", "pass", Role::Player)
        .build()
        .await;

    app.login_as("player", "pass").await;
    let csrf = app.csrf_token_from("/quma/").await;

    let resp = app
        .post_form("/quma/server/start", &format!("csrf_token={}", csrf))
        .await;
    assert_eq!(resp.status(), StatusCode::FORBIDDEN);
}

#[actix_web::test]
async fn server_stop_requires_capability() {
    let mut app = TestAppBuilder::new()
        .with_user("player", "pass", Role::Player)
        .build()
        .await;

    app.login_as("player", "pass").await;
    let csrf = app.csrf_token_from("/quma/").await;

    let resp = app
        .post_form("/quma/server/stop", &format!("csrf_token={}", csrf))
        .await;
    assert_eq!(resp.status(), StatusCode::FORBIDDEN);
}
