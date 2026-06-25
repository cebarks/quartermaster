mod common;

use actix_web::http::StatusCode;
use common::TestAppBuilder;

#[actix_web::test]
async fn headless_list_requires_auth() {
    let mut app = TestAppBuilder::new().build().await;

    let resp = app.get("/quma/headless").await;
    assert_eq!(resp.status(), StatusCode::SEE_OTHER);
}

#[actix_web::test]
async fn headless_list_loads() {
    let mut app = TestAppBuilder::new()
        .with_user("admin", "pass", "admin")
        .build()
        .await;

    app.login_as("admin", "pass").await;

    let resp = app.get("/quma/headless").await;
    assert_eq!(resp.status(), StatusCode::OK);
}
