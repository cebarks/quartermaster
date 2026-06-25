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

#[actix_web::test]
async fn mod_archive_requires_invite_code() {
    let mut app = TestAppBuilder::new().build().await;

    let resp = app.get("/quma/join/mods.zip").await;
    assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
}

#[actix_web::test]
async fn mod_archive_returns_503_when_no_mods_installed() {
    let mut app = TestAppBuilder::new()
        .with_invite("quma-testcode1", None)
        .build()
        .await;

    let resp = app.get("/quma/join/mods.zip?code=quma-testcode1").await;
    assert_eq!(resp.status(), StatusCode::SERVICE_UNAVAILABLE);
}

#[actix_web::test]
async fn bootstrap_bash_requires_invite_code() {
    let mut app = TestAppBuilder::new().build().await;

    let resp = app.get("/quma/join/bootstrap.sh").await;
    assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
}

#[actix_web::test]
async fn bootstrap_bash_returns_script_with_valid_code() {
    let mut app = TestAppBuilder::new()
        .with_invite("quma-testcode1", None)
        .with_external_url("https://test.example.com:9190")
        .build()
        .await;

    let resp = app.get("/quma/join/bootstrap.sh?code=quma-testcode1").await;
    assert!(resp.status().is_success());

    let content_type = resp
        .headers()
        .get("content-type")
        .unwrap()
        .to_str()
        .unwrap();
    assert_eq!(content_type, "text/x-shellscript");

    let body = actix_web::test::read_body(resp).await;
    let script = std::str::from_utf8(&body).unwrap();
    assert!(script.contains("#!/usr/bin/env bash"));
    assert!(script.contains("test.example.com:9190"));
    assert!(script.contains("quma-testcode1"));
}

#[actix_web::test]
async fn bootstrap_powershell_returns_script_with_valid_code() {
    let mut app = TestAppBuilder::new()
        .with_invite("quma-testcode1", None)
        .with_external_url("https://test.example.com:9190")
        .build()
        .await;

    let resp = app
        .get("/quma/join/bootstrap.ps1?code=quma-testcode1")
        .await;
    assert!(resp.status().is_success());

    let body = actix_web::test::read_body(resp).await;
    let script = std::str::from_utf8(&body).unwrap();
    assert!(script.contains("$ErrorActionPreference"));
    assert!(script.contains("test.example.com:9190"));
}

#[actix_web::test]
async fn bootstrap_bash_returns_503_without_external_url() {
    let mut app = TestAppBuilder::new()
        .with_invite("quma-testcode1", None)
        .build()
        .await;

    let resp = app.get("/quma/join/bootstrap.sh?code=quma-testcode1").await;
    assert_eq!(resp.status(), StatusCode::SERVICE_UNAVAILABLE);
}
