#[path = "web_helpers.rs"]
mod web_helpers;

use actix_web::http::StatusCode;
use spt_quartermaster::db::users::Role;
use web_helpers::TestAppBuilder;

#[actix_web::test]
async fn headless_list_requires_auth() {
    let mut app = TestAppBuilder::new().build().await;

    let resp = app.get("/quma/headless").await;
    assert_eq!(resp.status(), StatusCode::SEE_OTHER);
}

#[actix_web::test]
async fn headless_list_loads() {
    let mut app = TestAppBuilder::new()
        .with_user("admin", "pass", Role::Admin)
        .build()
        .await;

    app.login_as("admin", "pass").await;

    let resp = app.get("/quma/headless").await;
    assert_eq!(resp.status(), StatusCode::OK);
}

#[actix_web::test]
async fn legacy_clients_url_redirects() {
    let mut app = TestAppBuilder::new()
        .with_user("admin", "pass", Role::Admin)
        .build()
        .await;

    app.login_as("admin", "pass").await;

    let resp = app.get("/quma/clients").await;
    assert_eq!(resp.status(), StatusCode::MOVED_PERMANENTLY);

    let location = resp.headers().get("location").unwrap().to_str().unwrap();
    assert_eq!(location, "/quma/headless");
}

#[actix_web::test]
async fn legacy_clients_n_redirects() {
    let mut app = TestAppBuilder::new()
        .with_user("admin", "pass", Role::Admin)
        .build()
        .await;

    app.login_as("admin", "pass").await;

    let resp = app.get("/quma/clients/1").await;
    assert_eq!(resp.status(), StatusCode::MOVED_PERMANENTLY);

    let location = resp.headers().get("location").unwrap().to_str().unwrap();
    assert_eq!(location, "/quma/headless/1");
}
