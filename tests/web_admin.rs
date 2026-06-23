mod common;

use actix_web::http::StatusCode;
use actix_web::test;
use common::TestAppBuilder;

// -- Access Control Tests --

#[actix_web::test]
async fn admin_page_requires_auth() {
    let mut app = TestAppBuilder::new().build().await;
    let resp = app.get("/quma/admin").await;
    assert_eq!(resp.status(), StatusCode::SEE_OTHER);
    let location = resp.headers().get("location").unwrap().to_str().unwrap();
    assert_eq!(location, "/quma/login");
}

#[actix_web::test]
async fn admin_page_requires_admin_role() {
    let mut app = TestAppBuilder::new()
        .with_user("player", "password", "player")
        .build()
        .await;

    app.login_as("player", "password").await;
    let resp = app.get("/quma/admin").await;
    assert_eq!(resp.status(), StatusCode::FORBIDDEN);
}

#[actix_web::test]
async fn admin_page_accessible_by_admin() {
    let mut app = TestAppBuilder::new()
        .with_user("admin", "password", "admin")
        .build()
        .await;

    app.login_as("admin", "password").await;
    let resp = app.get("/quma/admin").await;
    assert_eq!(resp.status(), StatusCode::OK);
}

#[actix_web::test]
async fn admin_api_requires_admin_role() {
    let mut app = TestAppBuilder::new()
        .with_user("moderator", "password", "moderator")
        .build()
        .await;

    app.login_as("moderator", "password").await;
    let resp = app.get("/quma/api/admin/users").await;
    assert_eq!(resp.status(), StatusCode::FORBIDDEN);
}

// -- User Management Tests --

#[actix_web::test]
async fn admin_change_role() {
    let mut app = TestAppBuilder::new()
        .with_user("admin", "password", "admin")
        .with_user("player", "password", "player")
        .build()
        .await;

    app.login_as("admin", "password").await;

    // Get the player user ID from DB
    let db = app.db.lock();
    let player = db
        .get_user_by_username("player")
        .unwrap()
        .expect("player user should exist");
    let player_id = player.id;
    drop(db);

    // Get CSRF token
    let csrf_token = app.csrf_token_from("/quma/admin").await;

    // Change player role to moderator
    let form_body = format!(
        "role=moderator&csrf_token={}",
        urlencoding::encode(&csrf_token)
    );
    let resp = app
        .post_form(
            &format!("/quma/api/admin/users/{}/role", player_id),
            &form_body,
        )
        .await;
    assert_eq!(resp.status(), StatusCode::OK);

    // Verify role changed in DB
    let db = app.db.lock();
    let updated_player = db
        .get_user_by_username("player")
        .unwrap()
        .expect("player should still exist");
    assert_eq!(updated_player.role, "moderator");
}

#[actix_web::test]
async fn admin_cannot_self_demote() {
    let mut app = TestAppBuilder::new()
        .with_user("admin", "password", "admin")
        .build()
        .await;

    app.login_as("admin", "password").await;

    // Get admin user ID
    let db = app.db.lock();
    let admin = db
        .get_user_by_username("admin")
        .unwrap()
        .expect("admin user should exist");
    let admin_id = admin.id;
    drop(db);

    // Get CSRF token
    let csrf_token = app.csrf_token_from("/quma/admin").await;

    // Try to demote self
    let form_body = format!(
        "role=moderator&csrf_token={}",
        urlencoding::encode(&csrf_token)
    );
    let resp = app
        .post_form(
            &format!("/quma/api/admin/users/{}/role", admin_id),
            &form_body,
        )
        .await;

    // Should be forbidden
    assert_eq!(resp.status(), StatusCode::FORBIDDEN);
}

#[actix_web::test]
async fn admin_toggle_disable() {
    let mut app = TestAppBuilder::new()
        .with_user("admin", "password", "admin")
        .with_user("player", "password", "player")
        .build()
        .await;

    app.login_as("admin", "password").await;

    // Get player ID
    let db = app.db.lock();
    let player = db
        .get_user_by_username("player")
        .unwrap()
        .expect("player should exist");
    let player_id = player.id;
    assert!(!player.disabled, "player should start enabled");
    drop(db);

    // Get CSRF token
    let csrf_token = app.csrf_token_from("/quma/admin").await;

    // Disable the player
    let form_body = format!("csrf_token={}", urlencoding::encode(&csrf_token));
    let resp = app
        .post_form(
            &format!("/quma/api/admin/users/{}/disable", player_id),
            &form_body,
        )
        .await;
    assert_eq!(resp.status(), StatusCode::OK);

    // Verify user is disabled
    let db = app.db.lock();
    let disabled_player = db
        .get_user_by_id(player_id)
        .unwrap()
        .expect("player should exist");
    assert!(disabled_player.disabled, "player should be disabled");
    drop(db);

    // Get a fresh CSRF token (response may have updated it)
    let csrf_token = app.csrf_token_from("/quma/admin").await;

    // Re-enable the player
    let form_body = format!("csrf_token={}", urlencoding::encode(&csrf_token));
    let resp = app
        .post_form(
            &format!("/quma/api/admin/users/{}/disable", player_id),
            &form_body,
        )
        .await;
    assert_eq!(resp.status(), StatusCode::OK);

    // Verify user is re-enabled
    let db = app.db.lock();
    let enabled_player = db
        .get_user_by_id(player_id)
        .unwrap()
        .expect("player should exist");
    assert!(!enabled_player.disabled, "player should be re-enabled");
}

#[actix_web::test]
async fn admin_create_reset_token() {
    let mut app = TestAppBuilder::new()
        .with_user("admin", "password", "admin")
        .with_user("player", "password", "player")
        .build()
        .await;

    app.login_as("admin", "password").await;

    // Get player ID
    let db = app.db.lock();
    let player = db
        .get_user_by_username("player")
        .unwrap()
        .expect("player should exist");
    let player_id = player.id;
    drop(db);

    // Get CSRF token
    let csrf_token = app.csrf_token_from("/quma/admin").await;

    // Create reset token
    let form_body = format!("csrf_token={}", urlencoding::encode(&csrf_token));
    let resp = app
        .post_form(
            &format!("/quma/api/admin/users/{}/reset-password", player_id),
            &form_body,
        )
        .await;
    assert_eq!(resp.status(), StatusCode::OK);

    // Verify response contains a reset link
    let body = test::read_body(resp).await;
    let body_str = std::str::from_utf8(&body).unwrap();
    assert!(
        body_str.contains("/quma/reset-password?token="),
        "Response should contain reset link"
    );
}

// -- Invite Tests --

#[actix_web::test]
async fn admin_create_invite() {
    let mut app = TestAppBuilder::new()
        .with_user("admin", "password", "admin")
        .build()
        .await;

    app.login_as("admin", "password").await;

    // Get CSRF token
    let csrf_token = app.csrf_token_from("/quma/admin").await;

    // Create invite with never expiry
    let form_body = format!(
        "expiry=never&csrf_token={}",
        urlencoding::encode(&csrf_token)
    );
    let resp = app.post_form("/quma/api/admin/invites", &form_body).await;
    assert_eq!(resp.status(), StatusCode::OK);

    // Verify invite was created in DB
    let db = app.db.lock();
    let invites = db.list_invite_codes().unwrap();
    assert_eq!(invites.len(), 1, "should have exactly one invite");
    assert!(
        invites[0].invite.expires_at.is_none(),
        "invite should never expire"
    );
}
