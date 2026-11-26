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

    let body = actix_web::test::read_body(resp).await;
    let html = String::from_utf8_lossy(&body);
    assert!(html.contains("href=\"https://sp-mod.com\""));
    assert!(!html.contains("href=\"https://forge.sp-tarkov.com\""));
}

#[actix_web::test]
async fn dependency_preview_distinguishes_failure_from_empty_result() {
    use wiremock::matchers::{method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};
    for (response, unavailable, attempts) in [
        (ResponseTemplate::new(500), true, 4),
        (
            ResponseTemplate::new(200).set_body_string("malformed"),
            true,
            1,
        ),
        (
            ResponseTemplate::new(200).set_body_json(serde_json::json!({"data": []})),
            false,
            1,
        ),
    ] {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/mods/dependencies"))
            .respond_with(response)
            .expect(attempts)
            .mount(&server)
            .await;
        let mut app = TestAppBuilder::new()
            .with_user("admin", "password", "admin")
            .with_mock_server(server)
            .build()
            .await;
        app.login_as("admin", "password").await;
        let resp = app.get("/quma/api/mods/dep-tree?mod=791&ver=4.5.1").await;
        assert_eq!(resp.status(), StatusCode::OK);
        let body = actix_web::test::read_body(resp).await;
        let html = String::from_utf8_lossy(&body);
        assert_eq!(html.contains("Dependency preview unavailable"), unavailable);
        assert_eq!(html.contains("No dependencies."), !unavailable);
    }
}

#[actix_web::test]
async fn dependency_preview_reports_version_lookup_failure() {
    use wiremock::matchers::{method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/mod/791/versions"))
        .respond_with(ResponseTemplate::new(500))
        .expect(4)
        .mount(&server)
        .await;
    let mut app = TestAppBuilder::new()
        .with_user("admin", "password", "admin")
        .with_mock_server(server)
        .build()
        .await;
    app.login_as("admin", "password").await;
    let resp = app.get("/quma/api/mods/dep-tree?mod=791&ver_id=99").await;
    let body = actix_web::test::read_body(resp).await;
    let html = String::from_utf8_lossy(&body);
    assert!(html.contains("Dependency preview unavailable"));
    assert!(!html.contains("No dependencies."));
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

#[actix_web::test]
async fn svm_update_refreshes_selected_preset() {
    use std::io::{Cursor, Write};
    use wiremock::matchers::{method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    let server = MockServer::start().await;
    let forge_id = spt_quartermaster::svm::SVM_FORGE_ID;
    let mut zip = zip::ZipWriter::new(Cursor::new(Vec::new()));
    let options = zip::write::SimpleFileOptions::default();
    for (name, data) in [
        (
            "SPT/user/mods/SVM/Loader/loader.json",
            r#"{"CurrentlySelectedPreset":"Existing"}"#,
        ),
        (
            "SPT/user/mods/SVM/Presets/Existing.json",
            r#"{"sentinel":42}"#,
        ),
    ] {
        zip.start_file(name, options).unwrap();
        zip.write_all(data.as_bytes()).unwrap();
    }
    let bytes = zip.finish().unwrap().into_inner();
    Mock::given(method("GET"))
        .and(path(format!("/mod/{forge_id}/versions")))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "data": [{"id": 2, "version": "2.3", "link": format!("{}/download", server.uri())}]
        })))
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path("/mods/dependencies"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({"data": []})))
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path("/download"))
        .respond_with(ResponseTemplate::new(200).set_body_bytes(bytes))
        .mount(&server)
        .await;
    let mut app = TestAppBuilder::new()
        .with_user("admin", "pass", "admin")
        .with_mod(forge_id, "SVM", "2.2")
        .with_mock_server(server)
        .build()
        .await;
    let root = app.dirs().spt_server.clone();
    std::fs::write(
        root.join("SPT/SPT.Server.deps.json"),
        r#"{"libraries":{"SPT.Server/3.10.0":{}}}"#,
    )
    .unwrap();
    let dir = root.join("SPT/user/mods/SVM");
    std::fs::create_dir_all(dir.join("Loader")).unwrap();
    std::fs::write(
        dir.join("Loader/loader.json"),
        r#"{"CurrentlySelectedPreset":"null"}"#,
    )
    .unwrap();
    app.app_state.refresh_svm();
    assert_eq!(
        app.app_state
            .svm
            .read()
            .as_ref()
            .unwrap()
            .active_preset_name(),
        ""
    );
    let id = app
        .db
        .lock()
        .get_mod_by_forge_id(forge_id)
        .unwrap()
        .unwrap()
        .id;
    app.login_as("admin", "pass").await;
    let csrf = app.csrf_token_from(&format!("/quma/mods/{id}")).await;
    let resp = app
        .post_form(
            &format!("/quma/mods/{id}/update"),
            &format!("csrf_token={csrf}"),
        )
        .await;
    assert_eq!(resp.status(), StatusCode::SEE_OTHER);
    tokio::time::timeout(std::time::Duration::from_secs(10), async {
        loop {
            let tasks = app.app_state.tasks.task_views();
            if let Some(task) = tasks.first() {
                if !task.is_running {
                    assert!(task.message.contains("successfully"), "{}", task.message);
                    break;
                }
            }
            tokio::time::sleep(std::time::Duration::from_millis(10)).await;
        }
    })
    .await
    .expect("update did not finish");
    assert_eq!(
        app.app_state
            .svm
            .read()
            .as_ref()
            .unwrap()
            .active_preset_name(),
        "Existing"
    );
    assert_eq!(
        std::fs::read_to_string(dir.join("Presets/Existing.json")).unwrap(),
        r#"{"sentinel":42}"#
    );
}

#[actix_web::test]
async fn url_install_and_remove_refresh_svm_navigation() {
    use std::io::{Cursor, Write};
    use wiremock::matchers::{method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    let server = MockServer::start().await;
    let mut zip = zip::ZipWriter::new(Cursor::new(Vec::new()));
    let options = zip::write::SimpleFileOptions::default();
    for (name, data) in [
        (
            "SPT_Runtime/user/mods/SVM/Loader/loader.json",
            r#"{"CurrentlySelectedPreset":"null"}"#,
        ),
        (
            "SPT_Runtime/user/mods/SVM/ServerValueModifier.dll",
            "payload",
        ),
    ] {
        zip.start_file(name, options).unwrap();
        zip.write_all(data.as_bytes()).unwrap();
    }
    let bytes = zip.finish().unwrap().into_inner();
    Mock::given(method("GET"))
        .and(path("/SVM.zip"))
        .respond_with(ResponseTemplate::new(200).set_body_bytes(bytes))
        .mount(&server)
        .await;
    let url = format!("{}/SVM.zip", server.uri());
    let mut app = TestAppBuilder::new()
        .with_user("admin", "pass", "admin")
        .with_nested_layout()
        .with_mock_server(server)
        .build()
        .await;
    app.login_as("admin", "pass").await;
    let csrf = app.csrf_token_from("/quma/mods").await;
    let resp = app
        .post_form(
            "/quma/mods/install",
            &format!("mod_ref={url}&csrf_token={csrf}"),
        )
        .await;
    assert_eq!(resp.status(), StatusCode::SEE_OTHER);
    tokio::time::timeout(std::time::Duration::from_secs(10), async {
        loop {
            if let Some(task) = app.app_state.tasks.task_views().first() {
                if !task.is_running {
                    assert!(task.message.contains("installed"), "{}", task.message);
                    break;
                }
            }
            tokio::time::sleep(std::time::Duration::from_millis(10)).await;
        }
    })
    .await
    .expect("URL install did not finish");
    assert!(
        app.app_state.svm.read().is_some(),
        "SVM was not detected after URL install"
    );
    let installed = app
        .db
        .lock()
        .get_mod_by_name_or_slug("SVM")
        .unwrap()
        .unwrap();
    assert!(installed.forge_mod_id.is_none());
    let resp = app
        .post_form(
            &format!("/quma/mods/{}/remove", installed.id),
            &format!("csrf_token={csrf}"),
        )
        .await;
    assert_eq!(resp.status(), StatusCode::SEE_OTHER);
    assert!(
        app.app_state.svm.read().is_none(),
        "SVM remained detected after URL mod removal"
    );
}

#[actix_web::test]
#[ignore = "requires a usable FUSE overlay mount"]
async fn mounted_overlay_blocks_url_mod_remove_and_toggle() {
    let mut app = TestAppBuilder::new()
        .with_user("admin", "pass", "admin")
        .with_nested_layout()
        .build()
        .await;
    let relative = "SPT_Runtime/user/mods/Example/file.dll";
    let backing = app.dirs().mod_overlay().join(relative);
    std::fs::create_dir_all(backing.parent().unwrap()).unwrap();
    std::fs::write(&backing, b"original").unwrap();
    let id = {
        let db = app.db.lock();
        let id = db
            .insert_mod(
                None,
                None,
                "Example",
                None,
                "1.0",
                "url",
                Some("https://github.com/example/example/releases/download/v1.0/example.zip"),
                None,
            )
            .unwrap();
        db.insert_file(id, relative, None, None).unwrap();
        id
    };
    let overlay = app.dirs().spt_overlay();
    overlay.mount().unwrap();
    struct Mounted(spt_quartermaster::overlay::OverlayMount);
    impl Drop for Mounted {
        fn drop(&mut self) {
            self.0.unmount().unwrap();
        }
    }
    let mounted = Mounted(overlay);
    app.login_as("admin", "pass").await;
    let csrf = app.csrf_token_from("/quma/mods").await;
    for operation in ["remove", "toggle-disable"] {
        let resp = app
            .post_form(
                &format!("/quma/mods/{id}/{operation}"),
                &format!("csrf_token={csrf}"),
            )
            .await;
        assert_eq!(resp.status(), StatusCode::BAD_REQUEST, "{operation}");
        assert_eq!(std::fs::read(&backing).unwrap(), b"original");
        assert!(app.db.lock().get_mod(id).unwrap().is_some());
    }
    drop(mounted);
    let resp = app
        .post_form(
            &format!("/quma/mods/{id}/remove"),
            &format!("csrf_token={csrf}"),
        )
        .await;
    assert_eq!(resp.status(), StatusCode::SEE_OTHER);
    assert!(!backing.exists());
    assert!(app.db.lock().get_mod(id).unwrap().is_none());
}
