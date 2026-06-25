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

#[actix_web::test]
async fn join_page_requires_invite_code() {
    let mut app = TestAppBuilder::new().build().await;

    let resp = app.get("/quma/join").await;
    assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
}

#[actix_web::test]
async fn join_page_rejects_invalid_code() {
    let mut app = TestAppBuilder::new().build().await;

    let resp = app.get("/quma/join?code=invalid-code").await;
    assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
}

#[actix_web::test]
async fn join_page_loads_with_valid_code() {
    let mut app = TestAppBuilder::new()
        .with_invite("quma-testcode1", None)
        .with_external_url("https://test.example.com:9190")
        .build()
        .await;

    let resp = app.get("/quma/join?code=quma-testcode1").await;
    assert!(resp.status().is_success());
}

#[actix_web::test]
async fn join_page_returns_503_without_external_url() {
    let mut app = TestAppBuilder::new()
        .with_invite("quma-testcode1", None)
        .build()
        .await;

    let resp = app.get("/quma/join?code=quma-testcode1").await;
    // Default config has no external_url, so should 503
    assert_eq!(resp.status(), StatusCode::SERVICE_UNAVAILABLE);
}

#[actix_web::test]
async fn join_page_has_referrer_policy_header() {
    let mut app = TestAppBuilder::new()
        .with_invite("quma-testcode1", None)
        .build()
        .await;

    // Even on error responses, the header should be present
    let resp = app.get("/quma/join?code=quma-testcode1").await;
    assert_eq!(
        resp.headers()
            .get("referrer-policy")
            .unwrap()
            .to_str()
            .unwrap(),
        "no-referrer"
    );
}
