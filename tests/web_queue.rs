mod common;

use actix_web::http::StatusCode;
use common::TestAppBuilder;
use spt_quartermaster::db::users::Role;

#[actix_web::test]
async fn queue_content_requires_auth() {
    let mut app = TestAppBuilder::new().build().await;

    let resp = app.get("/quma/api/queue/content").await;
    assert_eq!(resp.status(), StatusCode::SEE_OTHER);
}

#[actix_web::test]
async fn queue_content_loads() {
    let mut app = TestAppBuilder::new()
        .with_user("admin", "pass", Role::Admin)
        .build()
        .await;

    app.login_as("admin", "pass").await;

    let resp = app.get("/quma/api/queue/content").await;
    assert_eq!(resp.status(), StatusCode::OK);
}

#[actix_web::test]
async fn apply_queue_requires_capability() {
    let mut app = TestAppBuilder::new()
        .with_user("player", "pass", Role::Player)
        .build()
        .await;

    app.login_as("player", "pass").await;
    let csrf = app.csrf_token_from("/quma/").await;

    let resp = app
        .post_form("/quma/queue/apply", &format!("csrf_token={}", csrf))
        .await;
    assert_eq!(resp.status(), StatusCode::FORBIDDEN);
}
