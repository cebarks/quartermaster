mod common;

use actix_web::http::StatusCode;
use common::TestAppBuilder;

#[actix_web::test]
async fn smoke_test_login_page_loads() {
    let mut app = TestAppBuilder::new().build().await;

    let resp = app.get("/quma/login").await;
    assert!(resp.status().is_success());
}

#[actix_web::test]
async fn smoke_test_unauthenticated_redirect() {
    let mut app = TestAppBuilder::new().build().await;

    let resp = app.get("/quma/").await;
    assert_eq!(resp.status(), StatusCode::SEE_OTHER);
    assert_eq!(
        resp.headers().get("location").unwrap().to_str().unwrap(),
        "/quma/login"
    );
}
