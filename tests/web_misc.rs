mod common;

use actix_web::http::StatusCode;
use common::TestAppBuilder;

// Static assets must be served without authentication so the login page can load CSS/JS
#[actix_web::test]
async fn assets_served_without_auth() {
    let mut app = TestAppBuilder::new().build().await;

    let resp = app.get("/quma/assets/style.css").await;
    assert_eq!(
        resp.status(),
        StatusCode::OK,
        "static assets must not require auth"
    );
}

// Browsers reject stylesheets/scripts without the correct Content-Type header
#[actix_web::test]
async fn assets_have_correct_content_type() {
    let mut app = TestAppBuilder::new().build().await;

    let resp = app.get("/quma/assets/style.css").await;
    let ct = resp
        .headers()
        .get("content-type")
        .expect("missing Content-Type on CSS");
    assert_eq!(ct, "text/css; charset=utf-8");

    let resp = app.get("/quma/assets/htmx.min.js").await;
    let ct = resp
        .headers()
        .get("content-type")
        .expect("missing Content-Type on JS");
    assert_eq!(ct, "application/javascript; charset=utf-8");
}

// Logs tests
#[actix_web::test]
async fn logs_page_requires_auth() {
    let mut app = TestAppBuilder::new().build().await;

    let resp = app.get("/quma/logs").await;
    assert_eq!(resp.status(), StatusCode::SEE_OTHER);
}

#[actix_web::test]
async fn logs_page_loads() {
    let mut app = TestAppBuilder::new()
        .with_user("admin", "pass", "admin")
        .build()
        .await;

    app.login_as("admin", "pass").await;

    let resp = app.get("/quma/logs").await;
    assert_eq!(resp.status(), StatusCode::OK);
}

#[actix_web::test]
async fn app_logs_json_requires_auth() {
    let mut app = TestAppBuilder::new().build().await;

    let resp = app.get("/quma/api/logs/app").await;
    assert_eq!(resp.status(), StatusCode::SEE_OTHER);
}

// Metrics tests
#[actix_web::test]
async fn metrics_page_requires_auth() {
    let mut app = TestAppBuilder::new().build().await;

    let resp = app.get("/quma/metrics").await;
    assert_eq!(resp.status(), StatusCode::SEE_OTHER);
}

#[actix_web::test]
async fn metrics_requires_server_control() {
    let mut app = TestAppBuilder::new()
        .with_user("player", "pass", "player")
        .build()
        .await;

    app.login_as("player", "pass").await;

    let resp = app.get("/quma/metrics").await;
    assert_eq!(resp.status(), StatusCode::FORBIDDEN);
}

#[actix_web::test]
async fn metrics_loads_for_admin() {
    let mut app = TestAppBuilder::new()
        .with_user("admin", "pass", "admin")
        .build()
        .await;

    app.login_as("admin", "pass").await;

    let resp = app.get("/quma/metrics").await;
    assert_eq!(resp.status(), StatusCode::OK);
}

// Tasks tests
#[actix_web::test]
async fn task_status_requires_auth() {
    let mut app = TestAppBuilder::new().build().await;

    let resp = app.get("/quma/api/tasks/status").await;
    assert_eq!(resp.status(), StatusCode::SEE_OTHER);
}

#[actix_web::test]
async fn task_status_loads() {
    let mut app = TestAppBuilder::new()
        .with_user("admin", "pass", "admin")
        .build()
        .await;

    app.login_as("admin", "pass").await;

    let resp = app.get("/quma/api/tasks/status").await;
    assert_eq!(resp.status(), StatusCode::OK);
}

// Profiles tests
#[actix_web::test]
async fn profile_page_requires_auth() {
    let mut app = TestAppBuilder::new().build().await;

    let resp = app.get("/quma/profiles/someone").await;
    assert_eq!(resp.status(), StatusCode::SEE_OTHER);
}

#[actix_web::test]
async fn profile_partials_require_auth() {
    let mut app = TestAppBuilder::new().build().await;

    let resp = app.get("/quma/api/profiles/someone/quests").await;
    assert_eq!(resp.status(), StatusCode::SEE_OTHER);
}

// Raids tests
#[actix_web::test]
async fn stats_page_requires_auth() {
    let mut app = TestAppBuilder::new().build().await;

    let resp = app.get("/quma/stats").await;
    assert_eq!(resp.status(), StatusCode::SEE_OTHER);
}

#[actix_web::test]
async fn stats_page_loads() {
    let mut app = TestAppBuilder::new()
        .with_user("admin", "pass", "admin")
        .build()
        .await;

    app.login_as("admin", "pass").await;

    let resp = app.get("/quma/stats").await;
    assert_eq!(resp.status(), StatusCode::OK);
}

#[actix_web::test]
async fn raids_partial_requires_auth() {
    let mut app = TestAppBuilder::new().build().await;

    let resp = app.get("/quma/api/raids/recent").await;
    assert_eq!(resp.status(), StatusCode::SEE_OTHER);
}

// SVM tests
#[actix_web::test]
async fn svm_page_requires_auth() {
    let mut app = TestAppBuilder::new().build().await;

    let resp = app.get("/quma/svm").await;
    assert_eq!(resp.status(), StatusCode::SEE_OTHER);
}

#[actix_web::test]
async fn svm_page_loads() {
    let mut app = TestAppBuilder::new()
        .with_user("admin", "pass", "admin")
        .build()
        .await;

    app.login_as("admin", "pass").await;

    let resp = app.get("/quma/svm").await;
    // SVM not installed in test env (svm: None), so handler returns 404
    assert!(
        resp.status() == StatusCode::OK || resp.status() == StatusCode::NOT_FOUND,
        "unexpected status: {}",
        resp.status()
    );
}

// Convoy tests
#[actix_web::test]
async fn convoy_page_requires_auth() {
    let mut app = TestAppBuilder::new().build().await;

    let resp = app.get("/quma/convoy").await;
    assert_eq!(resp.status(), StatusCode::SEE_OTHER);
}

#[actix_web::test]
async fn convoy_page_loads() {
    let mut app = TestAppBuilder::new()
        .with_user("admin", "pass", "admin")
        .build()
        .await;

    app.login_as("admin", "pass").await;

    let resp = app.get("/quma/convoy").await;
    assert_eq!(resp.status(), StatusCode::OK);
}

// Requests tests
#[actix_web::test]
async fn requests_tab_requires_auth() {
    let mut app = TestAppBuilder::new().build().await;

    let resp = app.get("/quma/api/mods/requests").await;
    assert_eq!(resp.status(), StatusCode::SEE_OTHER);
}

#[actix_web::test]
async fn requests_tab_loads() {
    let mut app = TestAppBuilder::new()
        .with_user("admin", "pass", "admin")
        .build()
        .await;

    app.login_as("admin", "pass").await;

    let resp = app.get("/quma/api/mods/requests").await;
    assert_eq!(resp.status(), StatusCode::OK);
}

// Groups tab tests
#[actix_web::test]
async fn mods_groups_tab_requires_auth() {
    let mut app = TestAppBuilder::new().build().await;

    let resp = app.get("/quma/api/mods/groups").await;
    assert_eq!(resp.status(), StatusCode::SEE_OTHER);
}

#[actix_web::test]
async fn mods_groups_tab_loads() {
    let mut app = TestAppBuilder::new()
        .with_user("admin", "pass", "admin")
        .build()
        .await;

    app.login_as("admin", "pass").await;

    let resp = app.get("/quma/api/mods/groups").await;
    assert_eq!(resp.status(), StatusCode::OK);
}

#[actix_web::test]
async fn svm_overlay_install_refresh_exposes_navigation_without_activation() {
    let mut app = TestAppBuilder::new()
        .with_user("admin", "pass", "admin")
        .with_nested_layout()
        .build()
        .await;
    let runtime = app.dirs().server_mod_relative_path("SVM").unwrap();
    let dir = app.dirs().mod_overlay().join(runtime);
    std::fs::create_dir_all(dir.join("Loader")).unwrap();
    std::fs::write(dir.join("ServerValueModifier.dll"), b"payload").unwrap();
    let loader = r#"{"CurrentlySelectedPreset":"null","Other":42}"#;
    std::fs::write(dir.join("Loader/loader.json"), loader).unwrap();
    app.app_state.refresh_svm();
    app.login_as("admin", "pass").await;
    let response = app.get("/quma/svm").await;
    assert_eq!(response.status(), StatusCode::OK);
    let html = actix_web::body::to_bytes(response.into_body())
        .await
        .unwrap();
    assert!(std::str::from_utf8(&html)
        .unwrap()
        .contains("No active preset"));
    assert_eq!(
        std::fs::read_to_string(dir.join("Loader/loader.json")).unwrap(),
        loader
    );
    assert!(!dir.join("Presets").exists());
}

#[actix_web::test]
async fn svm_edit_only_role_sees_svm_navigation() {
    use spt_quartermaster::db::rbac::Permission;

    let mut app = TestAppBuilder::new()
        .with_user("editor", "pass", "player")
        .with_nested_layout()
        .build()
        .await;
    let dir = app
        .dirs()
        .mod_overlay()
        .join(app.dirs().server_mod_relative_path("SVM").unwrap());
    std::fs::create_dir_all(dir.join("Loader")).unwrap();
    std::fs::write(dir.join("ServerValueModifier.dll"), b"payload").unwrap();
    std::fs::write(
        dir.join("Loader/loader.json"),
        r#"{"CurrentlySelectedPreset":"null"}"#,
    )
    .unwrap();
    app.app_state.refresh_svm();
    {
        let db = app.db.lock();
        db.create_role("svm_editor", "SVM Editor", &[Permission::SvmEdit])
            .unwrap();
        let user_id = db.get_user_by_username("editor").unwrap().unwrap().id;
        db.update_user_role(user_id, "svm_editor").unwrap();
    }
    app.login_as("editor", "pass").await;
    let resp = app.get("/quma/").await;
    assert_eq!(resp.status(), StatusCode::OK);
    let html = String::from_utf8(actix_web::test::read_body(resp).await.to_vec()).unwrap();
    assert!(html.contains("href=\"/quma/svm\""));
    assert_eq!(app.get("/quma/svm").await.status(), StatusCode::OK);
}

#[actix_web::test]
#[ignore = "requires a usable FUSE overlay mount"]
async fn svm_mounted_overlay_web_create_activate_and_save() {
    let mut app = TestAppBuilder::new()
        .with_user("admin", "pass", "admin")
        .with_nested_layout()
        .build()
        .await;
    let rel = app.dirs().server_mod_relative_path("SVM").unwrap();
    let lower = app.dirs().mod_overlay().join(&rel);
    std::fs::create_dir_all(lower.join("Loader")).unwrap();
    std::fs::write(lower.join("ServerValueModifier.dll"), b"payload").unwrap();
    let loader = r#"{"CurrentlySelectedPreset":"null","Other":42}"#;
    std::fs::write(lower.join("Loader/loader.json"), loader).unwrap();
    let overlay = app.dirs().spt_overlay();
    overlay.mount().unwrap();
    struct Unmount(spt_quartermaster::overlay::OverlayMount);
    impl Drop for Unmount {
        fn drop(&mut self) {
            self.0.unmount().unwrap();
        }
    }
    let _mount = Unmount(overlay);
    app.app_state.refresh_svm();
    app.login_as("admin", "pass").await;
    let token = app.csrf_token_from("/quma/svm").await;
    let create = format!("csrf_token={token}&name=Custom");
    assert_eq!(
        app.post_form("/quma/svm/preset/create", &create)
            .await
            .status(),
        StatusCode::SEE_OTHER
    );
    assert_eq!(
        std::fs::read_to_string(lower.join("Loader/loader.json")).unwrap(),
        loader
    );
    let activate = format!("csrf_token={token}&preset=Custom");
    assert_eq!(
        app.post_form("/quma/svm/preset/switch", &activate)
            .await
            .status(),
        StatusCode::SEE_OTHER
    );
    let merged = app.dirs().spt_runtime().join(&rel);
    let active: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(merged.join("Loader/loader.json")).unwrap())
            .unwrap();
    assert_eq!(active["CurrentlySelectedPreset"], "Custom");
    assert_eq!(active["Other"], 42);
    let mut raids = spt_quartermaster::svm::SvmConfig::default().raids;
    raids.raid_time = 45;
    let mut body = serde_json::to_value(raids).unwrap();
    body["csrf_token"] = token.into();
    assert_eq!(
        app.post_json("/quma/api/svm/edit/raids", &body)
            .await
            .status(),
        StatusCode::OK
    );
    let preset: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(merged.join("Presets/Custom.json")).unwrap())
            .unwrap();
    assert_eq!(preset["Raids"]["RaidTime"], 45);
    assert_eq!(
        std::fs::read_to_string(lower.join("Loader/loader.json")).unwrap(),
        loader
    );
}

#[actix_web::test]
async fn svm_inactive_requires_explicit_activation() {
    let mut app = TestAppBuilder::new()
        .with_user("admin", "pass", "admin")
        .build()
        .await;
    let svm_dir = app.dirs().server_mods_dir().unwrap().join("SVM");
    std::fs::create_dir_all(svm_dir.join("Presets")).unwrap();
    std::fs::create_dir_all(svm_dir.join("Loader")).unwrap();
    std::fs::write(
        svm_dir.join("Loader/loader.json"),
        r#"{"CurrentlySelectedPreset":"null"}"#,
    )
    .unwrap();
    app.app_state.refresh_svm();
    app.login_as("admin", "pass").await;
    let nav = app.get("/quma/mods").await;
    let nav = String::from_utf8(
        actix_web::body::to_bytes(nav.into_body())
            .await
            .unwrap()
            .to_vec(),
    )
    .unwrap();
    assert!(nav.contains("/quma/svm"));
    let response = app.get("/quma/svm").await;
    assert_eq!(response.status(), StatusCode::OK);
    let html = String::from_utf8(
        actix_web::body::to_bytes(response.into_body())
            .await
            .unwrap()
            .to_vec(),
    )
    .unwrap();
    assert!(html.contains("No active preset"));
    assert!(!html.contains("Edit Configuration"));
    assert_eq!(
        app.get("/quma/svm/edit").await.status(),
        StatusCode::SEE_OTHER
    );
    assert_eq!(
        app.get("/quma/svm/view").await.status(),
        StatusCode::NOT_FOUND
    );
    let token = app.csrf_token_from("/quma/svm").await;
    assert_eq!(
        app.post_json(
            "/quma/api/svm/edit/raids",
            &serde_json::json!({"csrf_token": token}),
        )
        .await
        .status(),
        StatusCode::NOT_FOUND
    );
    let form = format!("csrf_token={token}&name=example");
    assert_eq!(
        app.post_form("/quma/svm/preset/create", &form)
            .await
            .status(),
        StatusCode::SEE_OTHER
    );
    let loader = std::fs::read_to_string(svm_dir.join("Loader/loader.json")).unwrap();
    assert!(loader.contains("null"));
    let form = format!("csrf_token={token}&preset=example");
    assert_eq!(
        app.post_form("/quma/svm/preset/switch", &form)
            .await
            .status(),
        StatusCode::SEE_OTHER
    );
    assert_eq!(app.get("/quma/svm/edit").await.status(), StatusCode::OK);
    std::fs::remove_dir_all(svm_dir).unwrap();
    app.app_state.refresh_svm();
    assert_eq!(app.get("/quma/svm").await.status(), StatusCode::NOT_FOUND);
}

#[actix_web::test]
async fn svm_inactive_player_cannot_edit_or_create() {
    let mut app = TestAppBuilder::new()
        .with_user("player", "pass", "player")
        .build()
        .await;
    let svm_dir = app.dirs().server_mods_dir().unwrap().join("SVM");
    std::fs::create_dir_all(svm_dir.join("Loader")).unwrap();
    std::fs::write(
        svm_dir.join("Loader/loader.json"),
        r#"{"CurrentlySelectedPreset":"null"}"#,
    )
    .unwrap();
    app.app_state.refresh_svm();
    app.login_as("player", "pass").await;
    assert_eq!(
        app.get("/quma/svm/view").await.status(),
        StatusCode::NOT_FOUND
    );
    assert_eq!(
        app.get("/quma/svm/edit").await.status(),
        StatusCode::FORBIDDEN
    );
    assert_eq!(
        app.post_form("/quma/svm/preset/create", "name=forbidden&csrf_token=x")
            .await
            .status(),
        StatusCode::FORBIDDEN
    );
    assert!(!svm_dir.join("Presets/forbidden.json").exists());
}
