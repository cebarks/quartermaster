#[path = "web_helpers.rs"]
mod web_helpers;

use actix_web::http::StatusCode;
use spt_quartermaster::db::users::Role;
use web_helpers::TestAppBuilder;

#[actix_web::test]
async fn dashboard_requires_auth() {
    let mut app = TestAppBuilder::new().build().await;

    let response = app.get("/quma/").await;

    assert_eq!(response.status(), StatusCode::SEE_OTHER);
    let location = response
        .headers()
        .get("Location")
        .expect("redirect should have Location header");
    assert_eq!(location, "/quma/login");
}

#[actix_web::test]
async fn dashboard_loads_for_authenticated_user() {
    let mut app = TestAppBuilder::new()
        .with_user("testuser", "password", Role::Player)
        .build()
        .await;

    app.login_as("testuser", "password").await;

    let response = app.get("/quma/").await;

    assert_eq!(response.status(), StatusCode::OK);
}

#[actix_web::test]
async fn dashboard_server_partial_requires_auth() {
    let mut app = TestAppBuilder::new().build().await;

    let response = app.get("/quma/api/dashboard/server").await;

    assert_eq!(response.status(), StatusCode::SEE_OTHER);
    let location = response
        .headers()
        .get("Location")
        .expect("redirect should have Location header");
    assert_eq!(location, "/quma/login");
}

#[actix_web::test]
async fn dashboard_partials_load() {
    let mut app = TestAppBuilder::new()
        .with_user("testuser", "password", Role::Player)
        .build()
        .await;

    app.login_as("testuser", "password").await;

    let server_response = app.get("/quma/api/dashboard/server").await;
    assert_eq!(server_response.status(), StatusCode::OK);

    let mods_response = app.get("/quma/api/dashboard/mods").await;
    assert_eq!(mods_response.status(), StatusCode::OK);

    let container_response = app.get("/quma/api/dashboard/container").await;
    assert_eq!(container_response.status(), StatusCode::OK);
}

#[actix_web::test]
async fn root_redirects_to_quma() {
    let mut app = TestAppBuilder::new().build().await;

    let response = app.get("/").await;

    assert_eq!(response.status(), StatusCode::FOUND);
    let location = response
        .headers()
        .get("Location")
        .expect("redirect should have Location header");
    assert_eq!(location, "/quma/");
}
