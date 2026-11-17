mod common;

use actix_web::{http::StatusCode, test};
use common::TestAppBuilder;

#[actix_web::test]
async fn server_actions_without_container_preserve_redirect_and_flash() {
    let mut app = TestAppBuilder::new()
        .with_user("admin", "pass", "admin")
        .build()
        .await;
    app.login_as("admin", "pass").await;

    for action in ["start", "stop", "restart"] {
        let csrf = app.csrf_token_from("/quma/").await;
        let response = app
            .post_form(
                &format!("/quma/server/{action}"),
                &format!("csrf_token={csrf}"),
            )
            .await;
        assert_eq!(response.status(), StatusCode::SEE_OTHER, "{action}");
        assert_eq!(response.headers().get("location").unwrap(), "/quma/");
        let dashboard = app.get("/quma/").await;
        let body = test::read_body(dashboard).await;
        assert!(
            std::str::from_utf8(&body)
                .unwrap()
                .contains("No server_container configured. Set it in quartermaster.toml."),
            "{action}"
        );
    }
}

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
        .with_user("player", "pass", "player")
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
        .with_user("player", "pass", "player")
        .build()
        .await;

    app.login_as("player", "pass").await;
    let csrf = app.csrf_token_from("/quma/").await;

    let resp = app
        .post_form("/quma/server/stop", &format!("csrf_token={}", csrf))
        .await;
    assert_eq!(resp.status(), StatusCode::FORBIDDEN);
}
