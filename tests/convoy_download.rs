#[allow(dead_code)]
mod common;
use actix_web::{http::StatusCode, test};
use common::{TestApp, TestAppBuilder};
use spt_quartermaster::config::ConvoyConfig;
use std::{
    io::{Cursor, Read},
    path::Path,
};
use xxhash_rust::xxh3::xxh3_64;
use zip::ZipArchive;

const RECEIPT: &str = "BepInEx/plugins/ConvoySmokeFixture/receipt.txt";
const CONTENT: &[u8] = b"Convoy fixture bytes\n\x00\xff";

fn checksum(bytes: &[u8]) -> String {
    format!("{:016x}", xxh3_64(bytes))
}

fn write_file(root: &Path, path: &str, bytes: &[u8]) {
    let file = root.join(path);
    std::fs::create_dir_all(file.parent().unwrap()).unwrap();
    std::fs::write(file, bytes).unwrap();
}

fn seed_client_mod(app: &TestApp) -> i64 {
    app.set_convoy_config(Some(ConvoyConfig {
        enabled: true,
        ..ConvoyConfig::default()
    }));
    let db = app.db.lock();
    let id = db
        .insert_mod(
            None,
            None,
            "Convoy smoke fixture",
            None,
            "1.0.0",
            "local",
            None,
            None,
        )
        .unwrap();
    db.insert_file(
        id,
        RECEIPT,
        Some(&checksum(CONTENT)),
        Some(CONTENT.len() as i64),
    )
    .unwrap();
    id
}

#[actix_web::test]
async fn managed_dependency_publisher_delivery_setup_and_cache() {
    let app = TestAppBuilder::new().with_nested_layout().build().await;
    let id = seed_client_mod(&app);
    let managed = "EscapeFromTarkov_Data/Managed/Unity.VectorGraphics.dll";
    let bridge = "EscapeFromTarkov_Data/Managed/Unity.InternalAPIEngineBridge.003.dll";
    write_file(&app.dirs().mod_overlay(), RECEIPT, CONTENT);
    for path in [managed, bridge] {
        write_file(&app.dirs().mod_overlay(), path, CONTENT);
        app.db
            .lock()
            .insert_file(id, path, Some(&checksum(CONTENT)), None)
            .unwrap();
    }
    for path in [
        "SPT_Runtime/user/profiles/private.json",
        "EscapeFromTarkov_Data/Managed/not-a-dll.json",
        "private.txt",
    ] {
        write_file(&app.dirs().mod_overlay(), path, b"never public");
        app.db
            .lock()
            .insert_file(id, path, Some(&checksum(b"never public")), None)
            .unwrap();
    }
    write_file(
        &app.dirs().spt_server,
        "EscapeFromTarkov_Data/Managed/UntrackedGame.dll",
        b"game",
    );
    // Independent setup ZIP includes package dependencies and excludes server files by default.
    let tracked = app.db.lock().get_all_enabled_mod_files().unwrap();
    let filtered =
        spt_quartermaster::web::mod_zip_cache::filter_setup_zip_files(tracked, &Default::default());
    let setup = app.dirs().spt_server.join("fixture-setup.zip");
    spt_quartermaster::web::mod_zip_cache::build_mod_zip_to_file(
        &app.dirs(),
        &filtered,
        &setup,
        true,
    )
    .unwrap();
    let mut zip = ZipArchive::new(std::fs::File::open(setup).unwrap()).unwrap();
    assert!(zip.by_name(managed).is_ok());
    assert!(zip.by_name(bridge).is_ok());
    assert!(zip
        .by_name("SPT_Runtime/user/profiles/private.json")
        .is_err());
}

#[actix_web::test]
async fn v2_cutover_and_setup_independence() {
    use spt_quartermaster::convoy::publisher::{PublicationStatus, PublishMode};

    for governed in [false, true] {
        // A fresh root and governor for every cell: no background cache or quota
        // can leak in from the preceding delivery configuration.
        for (enabled, broken) in [(false, false), (true, true), (true, false)] {
            let mut app = TestAppBuilder::new()
                .with_nested_layout()
                .with_user("operator", "fixture-password", "admin")
                .with_user("player", "fixture-password", "player")
                .build()
                .await;
            if governed {
                app.enable_fixture_governor();
            }
            let id = seed_client_mod(&app);
            let extra = "extras/config.dat";
            let expected = [(RECEIPT, CONTENT), (extra, &b"setup only"[..])];
            write_file(&app.dirs().mod_overlay(), RECEIPT, CONTENT);
            write_file(&app.dirs().mod_overlay(), extra, expected[1].1);
            app.db
                .lock()
                .insert_file(id, extra, Some(&checksum(expected[1].1)), Some(10))
                .unwrap();
            app.set_convoy_config(Some(ConvoyConfig {
                enabled,
                max_store_bytes: if broken { 0 } else { 1024 },
                ..Default::default()
            }));
            let publisher = app.app_state.convoy_publisher.clone();
            let mut status = publisher.subscribe();
            publisher.refresh(PublishMode::Managed);
            if enabled {
                tokio::time::timeout(
                    std::time::Duration::from_secs(10),
                    status.wait_for(|s| {
                        matches!(
                            s,
                            PublicationStatus::Ready { .. } | PublicationStatus::Failed { .. }
                        )
                    }),
                )
                .await
                .unwrap()
                .unwrap();
            }
            let catalog = app.get("/quma/convoy/v2/catalog").await;
            if enabled && !broken {
                assert_eq!(catalog.status(), StatusCode::OK);
                let json: serde_json::Value = test::read_body_json(catalog).await;
                assert_eq!(json["protocol"], 2);
                let files = json["groups"][0]["mods"][0]["files"].as_array().unwrap();
                assert_eq!(files.len(), 1);
                assert_eq!(files[0]["path"], RECEIPT);
            } else {
                assert_eq!(catalog.status(), StatusCode::SERVICE_UNAVAILABLE);
                let json: serde_json::Value = test::read_body_json(catalog).await;
                assert_eq!(
                    json["code"],
                    if broken { "publish_failed" } else { "disabled" }
                );
            }
            for request in [
                test::TestRequest::get().uri("/quma/convoy/catalog"),
                test::TestRequest::post().uri("/quma/convoy/download"),
                test::TestRequest::get().uri(&format!("/quma/convoy/mod/{id}/archive")),
            ] {
                let response = app.send_raw(request.to_request()).await;
                assert_eq!(
                    response.status(),
                    if enabled {
                        StatusCode::GONE
                    } else {
                        StatusCode::SERVICE_UNAVAILABLE
                    }
                );
                assert_eq!(response.headers().get("Cache-Control").unwrap(), "no-store");
                let json: serde_json::Value = test::read_body_json(response).await;
                assert_eq!(
                    json["code"],
                    if enabled {
                        "upgrade_required"
                    } else {
                        "disabled"
                    }
                );
                if enabled {
                    assert_eq!(
                        json["instructions"],
                        "https://github.com/cebarks/convoy#installation"
                    );
                }
            }

            let cache = app.app_state.mod_zip_cache.clone();
            for warm in [false, true] {
                if warm {
                    cache.invalidate();
                    tokio::time::timeout(std::time::Duration::from_secs(10), async {
                        while cache.get().is_none() {
                            tokio::time::sleep(std::time::Duration::from_millis(10)).await;
                        }
                    })
                    .await
                    .unwrap();
                }
                assert_eq!(cache.get().is_some(), warm);
                let before = publisher.status();
                let response = app.get("/quma/setup/mods.zip").await;
                assert_eq!(response.status(), StatusCode::OK);
                assert_eq!(
                    response.headers().contains_key("ETag"),
                    warm,
                    "warm uses NamedFile; cold builds in the handler"
                );
                let body = test::read_body(response).await;
                if warm {
                    assert_eq!(body.as_ref(), std::fs::read(cache.get().unwrap()).unwrap());
                }
                let mut zip = ZipArchive::new(Cursor::new(body)).unwrap();
                assert_eq!(zip.len(), expected.len());
                for (path, bytes) in expected {
                    let mut got = Vec::new();
                    zip.by_name(path).unwrap().read_to_end(&mut got).unwrap();
                    assert_eq!(got, bytes);
                }
                assert_eq!(publisher.status(), before);
                println!(
                    "governed={governed} publisher={before:?} setup warm={warm} exact {:?}",
                    expected.map(|(path, bytes)| (path, bytes.len()))
                );
            }
            let report = app.send_raw(test::TestRequest::post().uri("/quma/convoy/report")
                .set_json(serde_json::json!({"aid":"cutover-fixture","result":"updated","client_version":"fixture"})).to_request()).await;
            assert_eq!(report.status(), StatusCode::OK);
            let reports = app.db.lock().get_latest_sync_reports().unwrap();
            assert_eq!(reports.len(), 1);
            assert_eq!(reports[0].aid, "cutover-fixture");
            assert_eq!(reports[0].result, "updated");

            if enabled && !broken {
                let rehash = "/quma/convoy/rehash";
                app.login_as("player", "fixture-password").await;
                let token = app.csrf_token_from("/quma/").await;
                assert_eq!(
                    app.send_authenticated(
                        test::TestRequest::post()
                            .uri(rehash)
                            .insert_header(("x-csrf-token", token.as_str()))
                    )
                    .await
                    .status(),
                    StatusCode::FORBIDDEN
                );
                // Independently validate that *the identical player session and token*
                // pass the real CSRF check on a player-accessible action.
                let logout = app
                    .post_form(
                        "/quma/logout",
                        &format!("csrf_token={}", urlencoding::encode(&token)),
                    )
                    .await;
                assert_eq!(logout.status(), StatusCode::SEE_OTHER);
                assert_eq!(logout.headers().get("Location").unwrap(), "/quma/login");
                app.login_as("operator", "fixture-password").await;
                assert_eq!(
                    app.send_authenticated(test::TestRequest::post().uri(rehash))
                        .await
                        .status(),
                    StatusCode::FORBIDDEN
                );
                let token = app.csrf_token_from("/quma/convoy?tab=settings").await;
                assert_eq!(
                    app.send_authenticated(
                        test::TestRequest::post()
                            .uri(rehash)
                            .insert_header(("x-csrf-token", token))
                    )
                    .await
                    .status(),
                    StatusCode::ACCEPTED
                );
                println!("governed={governed} player valid proof 403 (same proof logs out), admin 403 without proof / 202 with proof");
            }
            if !enabled {
                // Exhaust only this fixture's setup quota: a wrong governor branch
                // cannot produce the expected contrast even if the ZIP looks right.
                for _ in 0..6 {
                    assert_eq!(
                        app.get("/quma/setup/mods.zip").await.status(),
                        StatusCode::OK
                    );
                }
                let actual = app.get("/quma/setup/mods.zip").await.status();
                assert_eq!(
                    actual,
                    if governed {
                        StatusCode::TOO_MANY_REQUESTS
                    } else {
                        StatusCode::OK
                    }
                );
                println!("governed={governed} ninth setup GET: {actual}");
            }
        }
    }
}
