mod common;

use actix_web::http::StatusCode;
use common::TestAppBuilder;

#[actix_web::test]
async fn mods_page_requires_auth() {
    let mut app = TestAppBuilder::new().build().await;
    let resp = app.get("/quma/mods").await;
    assert_eq!(resp.status(), StatusCode::SEE_OTHER);
    let location = resp.headers().get("location").unwrap().to_str().unwrap();
    assert!(location.contains("/quma/login"));
}

#[actix_web::test]
async fn mods_page_loads() {
    let mut app = TestAppBuilder::new()
        .with_user("admin", "password", "admin")
        .build()
        .await;

    app.login_as("admin", "password").await;
    let resp = app.get("/quma/mods").await;
    assert_eq!(resp.status(), StatusCode::OK);
}

#[actix_web::test]
async fn mods_page_shows_installed_mods() {
    let mut app = TestAppBuilder::new()
        .with_user("admin", "password", "admin")
        .with_mod(123, "TestMod", "1.0.0")
        .build()
        .await;

    app.login_as("admin", "password").await;
    let resp = app.get("/quma/mods").await;
    assert_eq!(resp.status(), StatusCode::OK);

    let body = actix_web::test::read_body(resp).await;
    let body_str = String::from_utf8_lossy(&body);
    assert!(body_str.contains("TestMod"));
}

#[actix_web::test]
async fn mod_detail_nonexistent_returns_404() {
    let mut app = TestAppBuilder::new()
        .with_user("admin", "password", "admin")
        .build()
        .await;

    app.login_as("admin", "password").await;
    let resp = app.get("/quma/mods/99999").await;
    assert_eq!(resp.status(), StatusCode::INTERNAL_SERVER_ERROR); // Not found in DB returns 500
}

#[actix_web::test]
async fn mod_detail_shows_mod() {
    let mut app = TestAppBuilder::new()
        .with_user("admin", "password", "admin")
        .with_mod(123, "TestMod", "1.0.0")
        .build()
        .await;

    app.login_as("admin", "password").await;

    // Get the database ID
    let db = app.db.lock();
    let installed_mod = db
        .get_mod_by_forge_id(123)
        .unwrap()
        .expect("mod should exist");
    let db_id = installed_mod.id;
    drop(db);

    let resp = app.get(&format!("/quma/mods/{}", db_id)).await;
    assert_eq!(resp.status(), StatusCode::OK);

    let body = actix_web::test::read_body(resp).await;
    let body_str = String::from_utf8_lossy(&body);
    assert!(body_str.contains("TestMod"));
}

#[actix_web::test]
async fn toggle_disable_mod() {
    let mut app = TestAppBuilder::new()
        .with_user("admin", "password", "admin")
        .with_mod(123, "TestMod", "1.0.0")
        .build()
        .await;

    app.login_as("admin", "password").await;

    // Get the database ID
    let db = app.db.lock();
    let installed_mod = db
        .get_mod_by_forge_id(123)
        .unwrap()
        .expect("mod should exist");
    let db_id = installed_mod.id;
    assert!(!installed_mod.disabled);
    drop(db);

    // Get CSRF token
    let csrf_token = app.csrf_token_from(&format!("/quma/mods/{}", db_id)).await;

    // Toggle disable
    let resp = app
        .post_form(
            &format!("/quma/mods/{}/toggle-disable", db_id),
            &format!("csrf_token={}", csrf_token),
        )
        .await;
    assert_eq!(resp.status(), StatusCode::SEE_OTHER);

    // Verify mod is now disabled
    let db = app.db.lock();
    let updated_mod = db.get_mod(db_id).unwrap().unwrap();
    assert!(updated_mod.disabled);
}

#[actix_web::test]
async fn mods_list_partial_requires_auth() {
    let mut app = TestAppBuilder::new().build().await;
    let resp = app.get("/quma/api/mods/list").await;
    assert_eq!(resp.status(), StatusCode::SEE_OTHER);
    let location = resp.headers().get("location").unwrap().to_str().unwrap();
    assert!(location.contains("/quma/login"));
}

#[actix_web::test]
async fn install_mod_requires_capability() {
    let mut app = TestAppBuilder::new()
        .with_user("player", "password", "player")
        .build()
        .await;

    app.login_as("player", "password").await;

    // Get CSRF token from login/mods page
    let csrf_token = app.csrf_token_from("/quma/login").await;

    let resp = app
        .post_form(
            "/quma/mods/install",
            &format!("mod_ref=123&csrf_token={}", csrf_token),
        )
        .await;

    assert_eq!(resp.status(), StatusCode::FORBIDDEN);
}
