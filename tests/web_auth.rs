mod common;

use actix_web::http::StatusCode;
use actix_web::test;
use common::{extract_csrf_token, TestAppBuilder};

// ── Login tests ───────────────────────────────────────────────────────

#[actix_web::test]
async fn login_page_renders() {
    let mut app = TestAppBuilder::new().build().await;
    let resp = app.get("/quma/login").await;
    assert_eq!(resp.status(), StatusCode::OK);
    let body_bytes = test::read_body(resp).await;
    let body = String::from_utf8_lossy(&body_bytes);
    assert!(extract_csrf_token(&body).is_some());
}

#[actix_web::test]
async fn login_valid_credentials_redirects() {
    let mut app = TestAppBuilder::new()
        .with_user("testuser", "password123", "player")
        .build()
        .await;

    app.login_as("testuser", "password123").await;

    // After successful login, accessing dashboard should return 200
    let resp = app.get("/quma/").await;
    assert_eq!(resp.status(), StatusCode::OK);
}

#[actix_web::test]
async fn login_wrong_password() {
    let mut app = TestAppBuilder::new()
        .with_user("testuser", "correctpass", "player")
        .build()
        .await;

    let csrf = app.csrf_token_from("/quma/login").await;
    let form_data = format!(
        "username=testuser&password=wrongpass&csrf_token={}",
        urlencoding::encode(&csrf)
    );
    let resp = app.post_form("/quma/login", &form_data).await;

    assert_eq!(resp.status(), StatusCode::OK);
    let body_bytes = test::read_body(resp).await;
    let body = String::from_utf8_lossy(&body_bytes);
    assert!(body.contains("Invalid username or password"));
}

#[actix_web::test]
async fn login_nonexistent_user() {
    let mut app = TestAppBuilder::new().build().await;

    let csrf = app.csrf_token_from("/quma/login").await;
    let form_data = format!(
        "username=nonexistent&password=anypass&csrf_token={}",
        urlencoding::encode(&csrf)
    );
    let resp = app.post_form("/quma/login", &form_data).await;

    assert_eq!(resp.status(), StatusCode::OK);
    let body_bytes = test::read_body(resp).await;
    let body = String::from_utf8_lossy(&body_bytes);
    assert!(body.contains("Invalid username or password"));
}

#[actix_web::test]
async fn login_missing_csrf() {
    let mut app = TestAppBuilder::new()
        .with_user("testuser", "password123", "player")
        .build()
        .await;

    let form_data = "username=testuser&password=password123&csrf_token=badtoken";
    let resp = app.post_form("/quma/login", form_data).await;

    assert_eq!(resp.status(), StatusCode::FORBIDDEN);
}

#[actix_web::test]
async fn login_disabled_user() {
    let mut app = TestAppBuilder::new()
        .with_user("disableduser", "password123", "player")
        .build()
        .await;

    // Disable the user
    let user_id = {
        let db = app.db.lock();
        let user = db.get_user_by_username("disableduser").unwrap().unwrap();
        user.id
    };
    {
        let db = app.db.lock();
        db.set_user_disabled(user_id, true).unwrap();
    }

    let csrf = app.csrf_token_from("/quma/login").await;
    let form_data = format!(
        "username=disableduser&password=password123&csrf_token={}",
        urlencoding::encode(&csrf)
    );
    let resp = app.post_form("/quma/login", &form_data).await;

    assert_eq!(resp.status(), StatusCode::OK);
    let body_bytes = test::read_body(resp).await;
    let body = String::from_utf8_lossy(&body_bytes);
    assert!(body.contains("Invalid username or password"));
}

// ── Logout ────────────────────────────────────────────────────────────

#[actix_web::test]
async fn logout_clears_session() {
    let mut app = TestAppBuilder::new()
        .with_user("testuser", "password123", "player")
        .build()
        .await;

    app.login_as("testuser", "password123").await;

    // Get CSRF token from a page (we'll use login page for simplicity)
    let csrf = app.csrf_token_from("/quma/login").await;
    let form_data = format!("csrf_token={}", urlencoding::encode(&csrf));
    let resp = app.post_form("/quma/logout", &form_data).await;

    assert_eq!(resp.status(), StatusCode::SEE_OTHER);

    // After logout, accessing dashboard should redirect to login
    let resp = app.get("/quma/").await;
    assert_eq!(resp.status(), StatusCode::SEE_OTHER);
}

// ── Password Reset ────────────────────────────────────────────────────

#[actix_web::test]
async fn reset_password_no_token_returns_400() {
    let mut app = TestAppBuilder::new().build().await;
    let resp = app.get("/quma/reset-password").await;
    assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
    let body_bytes = test::read_body(resp).await;
    let body = String::from_utf8_lossy(&body_bytes);
    assert!(body.contains("invalid or has already been used"));
}

#[actix_web::test]
async fn reset_password_invalid_token_returns_400() {
    let mut app = TestAppBuilder::new().build().await;
    let resp = app.get("/quma/reset-password?token=badtoken").await;
    assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
    let body_bytes = test::read_body(resp).await;
    let body = String::from_utf8_lossy(&body_bytes);
    assert!(body.contains("invalid or has already been used"));
}

#[actix_web::test]
async fn reset_password_valid_token_renders_form() {
    let mut app = TestAppBuilder::new()
        .with_user("testuser", "oldpass123", "player")
        .build()
        .await;

    // Create reset token
    let user_id = {
        let db = app.db.lock();
        let user = db.get_user_by_username("testuser").unwrap().unwrap();
        user.id
    };
    let expires_at = (chrono::Utc::now() + chrono::Duration::hours(1)).to_rfc3339();
    {
        let db = app.db.lock();
        db.create_reset_token(user_id, "validtoken123", &expires_at)
            .unwrap();
    }

    let resp = app.get("/quma/reset-password?token=validtoken123").await;
    assert_eq!(resp.status(), StatusCode::OK);
    let body_bytes = test::read_body(resp).await;
    let body = String::from_utf8_lossy(&body_bytes);
    assert!(body.contains("validtoken123"));
}

#[actix_web::test]
async fn reset_password_submit_changes_password() {
    let mut app = TestAppBuilder::new()
        .with_user("testuser", "oldpass123", "player")
        .build()
        .await;

    // Create reset token
    let user_id = {
        let db = app.db.lock();
        let user = db.get_user_by_username("testuser").unwrap().unwrap();
        user.id
    };
    let expires_at = (chrono::Utc::now() + chrono::Duration::hours(1)).to_rfc3339();
    {
        let db = app.db.lock();
        db.create_reset_token(user_id, "validtoken123", &expires_at)
            .unwrap();
    }

    // Get CSRF token from reset password page
    let csrf = app
        .csrf_token_from("/quma/reset-password?token=validtoken123")
        .await;

    // Submit password reset
    let form_data = format!(
        "token=validtoken123&password=newpass123&password_confirm=newpass123&csrf_token={}",
        urlencoding::encode(&csrf)
    );
    let resp = app.post_form("/quma/reset-password", &form_data).await;

    assert_eq!(resp.status(), StatusCode::SEE_OTHER);

    // Verify we can login with new password
    app.login_as("testuser", "newpass123").await;
    let resp = app.get("/quma/").await;
    assert_eq!(resp.status(), StatusCode::OK);
}
