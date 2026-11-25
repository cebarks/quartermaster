mod common;

use actix_web::http::StatusCode;
use base64::Engine as _;
use common::TestAppBuilder;

fn embedded_bootstrap_config(script: &str) -> serde_json::Value {
    let marker = if script.starts_with("#!/usr/bin/env bash") {
        "base64.b64decode('"
    } else {
        "FromBase64String('"
    };
    let encoded = script
        .split_once(marker)
        .expect("embedded bootstrap configuration missing")
        .1
        .split('\'')
        .next()
        .unwrap();
    serde_json::from_slice(
        &base64::engine::general_purpose::STANDARD
            .decode(encoded)
            .unwrap(),
    )
    .unwrap()
}

#[actix_web::test]
async fn client_bootstrap_routes_embed_instance_contract_and_keep_auth() {
    for (path, archive_path, authenticated) in [
        (
            "/quma/join/bootstrap.sh?code=quma-bootstrap-contract",
            "/quma/join/mods.zip?code=quma-bootstrap-contract",
            false,
        ),
        (
            "/quma/join/bootstrap.ps1?code=quma-bootstrap-contract",
            "/quma/join/mods.zip?code=quma-bootstrap-contract",
            false,
        ),
        ("/quma/setup/bootstrap.sh", "/quma/setup/mods.zip", true),
        ("/quma/setup/bootstrap.ps1", "/quma/setup/mods.zip", true),
    ] {
        let mut app = TestAppBuilder::new()
            .with_user("bootstrap-player", "fixture-password-123", "player")
            .with_invite("quma-bootstrap-contract", None)
            .with_mod(2806, "Convoy", "0.6.1")
            .with_external_url("https://client.example.test:9190")
            .build()
            .await;
        if authenticated {
            assert_eq!(app.get(path).await.status(), StatusCode::SEE_OTHER);
            app.login_as("bootstrap-player", "fixture-password-123")
                .await;
        }
        let response = app.get(path).await;
        assert_eq!(response.status(), StatusCode::OK);
        let body = actix_web::test::read_body(response).await;
        let script = std::str::from_utf8(&body).unwrap();
        let config = embedded_bootstrap_config(script);
        assert_eq!(config["server_url"], "https://client.example.test:9190");
        assert_eq!(
            config["archive_url"],
            format!("https://client.example.test:9190{archive_path}")
        );
        assert_eq!(config["spt_version"], "3.10.0");
        assert_eq!(config["tarkov_version"], "0.16.0.xxxxx");
        assert!(config["required_files"]
            .as_array()
            .unwrap()
            .contains(&serde_json::json!("BepInEx/plugins/Convoy/Convoy.dll")));
        assert!(!script.contains("wine Fika-Installer.exe"));
        assert!(!script.contains("Invoke-WebRequest -Uri $FikaInstallerUrl"));
    }
}

#[actix_web::test]
async fn invite_archive_contains_full_filtered_instance_pack_cold_and_cached() {
    use std::io::Read;
    for cached in [false, true] {
        let mut app = TestAppBuilder::new()
            .with_nested_layout()
            .with_invite("quma-fullpack", None)
            .with_mod(2806, "Convoy", "0.6.1")
            .with_mod(999001, "Fika client fixture", "2.4.6")
            .build()
            .await;
        for (forge, relative, data) in [
            (
                2806,
                "BepInEx/plugins/Convoy/Convoy.dll",
                b"convoy fixture".as_slice(),
            ),
            (
                999001,
                "BepInEx/plugins/Fika/Fika.Core.dll",
                b"fika fixture".as_slice(),
            ),
            (
                999001,
                "SPT_Runtime/user/profiles/never-export.json",
                b"private fixture".as_slice(),
            ),
        ] {
            let destination = app.dirs().mod_overlay().join(relative);
            std::fs::create_dir_all(destination.parent().unwrap()).unwrap();
            std::fs::write(destination, data).unwrap();
            let db = app.db.lock();
            let id = db.get_mod_by_forge_id(forge).unwrap().unwrap().id;
            db.insert_file(id, relative, None, None).unwrap();
        }
        if cached {
            let warmup = app.get("/quma/setup/mods.zip").await;
            assert_eq!(warmup.status(), StatusCode::OK);
            let bytes = actix_web::test::read_body(warmup).await;
            let cache = app
                .dirs()
                .spt_server
                .join("quartermaster-cache/all-mods.zip");
            std::fs::create_dir_all(cache.parent().unwrap()).unwrap();
            std::fs::write(cache, bytes).unwrap();
        }
        let response = app.get("/quma/join/mods.zip?code=quma-fullpack").await;
        assert_eq!(response.status(), StatusCode::OK);
        let bytes = actix_web::test::read_body(response).await;
        let mut archive = zip::ZipArchive::new(std::io::Cursor::new(bytes)).unwrap();
        assert_eq!(
            archive.len(),
            2,
            "invite must include the complete client pack, never runtime profiles"
        );
        let mut data = String::new();
        archive
            .by_name("BepInEx/plugins/Fika/Fika.Core.dll")
            .unwrap()
            .read_to_string(&mut data)
            .unwrap();
        assert_eq!(data, "fika fixture");
    }
}

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
async fn join_page_renders_without_external_url() {
    let mut app = TestAppBuilder::new()
        .with_invite("quma-testcode1", None)
        .build()
        .await;

    let resp = app.get("/quma/join?code=quma-testcode1").await;
    // The join page is now a registration form and does not require external_url
    assert_eq!(resp.status(), StatusCode::OK);
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
async fn review_two_join_and_setup_fallback_export_overlay_only_files() {
    use std::io::Read;
    for endpoint in [
        "/quma/join/mods.zip?code=quma-testcode1",
        "/quma/setup/mods.zip",
    ] {
        let mut app = TestAppBuilder::new()
            .with_nested_layout()
            .with_invite("quma-testcode1", None)
            .with_mod(2806, "Convoy", "1.0")
            .build()
            .await;
        for (root, name, contents) in [
            (app.dirs().runtime_upper(), "upper.dll", "upper data"),
            (app.dirs().mod_overlay(), "mod.dll", "mod data"),
        ] {
            let relative = format!("BepInEx/plugins/{name}");
            let path = root.join(&relative);
            std::fs::create_dir_all(path.parent().unwrap()).unwrap();
            std::fs::write(path, contents).unwrap();
            let db = app.db.lock();
            let id = db.get_mod_by_forge_id(2806).unwrap().unwrap().id;
            db.insert_file(id, &relative, None, None).unwrap();
        }
        let response = app.get(endpoint).await;
        assert_eq!(response.status(), StatusCode::OK);
        let bytes = actix_web::test::read_body(response).await;
        let mut archive = zip::ZipArchive::new(std::io::Cursor::new(bytes)).unwrap();
        assert_eq!(archive.len(), 2);
        for (name, expected) in [("upper.dll", "upper data"), ("mod.dll", "mod data")] {
            let mut contents = String::new();
            archive
                .by_name(&format!("BepInEx/plugins/{name}"))
                .unwrap()
                .read_to_string(&mut contents)
                .unwrap();
            assert_eq!(contents, expected);
        }
    }
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
    let config = embedded_bootstrap_config(script);
    assert_eq!(config["server_url"], "https://test.example.com:9190");
    assert!(config["archive_url"]
        .as_str()
        .unwrap()
        .contains("quma-testcode1"));
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
    assert_eq!(
        embedded_bootstrap_config(script)["server_url"],
        "https://test.example.com:9190"
    );
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
