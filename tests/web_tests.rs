mod web_helpers;

use actix_web::test;
use spt_quartermaster::db::users::Role;
use web_helpers::TestAppBuilder;

#[actix_web::test]
async fn smoke_test_login_page_loads() {
    let mut app = TestAppBuilder::new().build().await;

    let resp = app.get("/quma/login").await;
    assert!(resp.status().is_success());

    let body = test::read_body(resp).await;
    let body_str = std::str::from_utf8(&body).unwrap();
    assert!(body_str.contains("Login"));
}

#[actix_web::test]
async fn smoke_test_unauthenticated_redirect() {
    let mut app = TestAppBuilder::new().build().await;

    let resp = app.get("/quma/").await;
    assert_eq!(resp.status().as_u16(), 303); // See Other redirect
    assert_eq!(
        resp.headers().get("location").unwrap().to_str().unwrap(),
        "/quma/login"
    );
}
