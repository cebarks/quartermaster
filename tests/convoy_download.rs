#[allow(dead_code)]
mod common;

use std::hash::{DefaultHasher, Hash, Hasher};
use std::io::{Cursor, Read};
use std::path::Path;

use actix_web::{http::StatusCode, test};
use serde_json::json;
use spt_quartermaster::config::ConvoyConfig;
use spt_quartermaster::convoy::{catalog::generate_catalog, download::clear_convoy_cache};
use xxhash_rust::xxh3::xxh3_64;
use zip::{ZipArchive, ZipWriter};

use common::{TestApp, TestAppBuilder};

const RECEIPT: &str = "BepInEx/plugins/ConvoySmokeFixture/receipt.txt";
const CONTENT: &[u8] = b"Convoy fixture bytes\n\x00\xff";
const MISSING: &str = "BepInEx/plugins/ConvoySmokeFixture/zz-missing.txt";

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

fn batch_request(id: i64, bundles_only: bool) -> actix_http::Request {
    test::TestRequest::post()
        .uri("/quma/convoy/download")
        .set_json(json!({"mods": [id], "bundles_only": bundles_only}))
        .to_request()
}

fn archive_requests(id: i64) -> [actix_http::Request; 2] {
    [
        batch_request(id, false),
        test::TestRequest::get()
            .uri(&format!("/quma/convoy/mod/{id}/archive"))
            .to_request(),
    ]
}

async fn assert_archives_match_catalog(app: &mut TestApp, id: i64) {
    let catalog = generate_catalog(
        &app.db.lock(),
        &app.dirs().spt_server,
        &ConvoyConfig::default(),
    )
    .unwrap();
    let entry = &catalog.groups[0].mods[0];
    assert_eq!(entry.id, id);
    let advertised_checksum = &entry.file_checksums[RECEIPT];

    for request in archive_requests(id) {
        let uri = request.uri().to_string();
        let response = app.send_raw(request).await;
        assert_eq!(response.status(), StatusCode::OK, "{uri}");
        let body = test::read_body(response).await;
        let mut zip = ZipArchive::new(Cursor::new(&body)).unwrap();
        assert_eq!(zip.len(), 1, "{uri} returned a {}-byte ZIP", body.len());
        let mut contents = Vec::new();
        zip.by_name(RECEIPT)
            .unwrap()
            .read_to_end(&mut contents)
            .unwrap();
        assert_eq!(contents, CONTENT, "{uri}");
        assert_eq!(checksum(&contents), *advertised_checksum, "{uri}");
        // The second endpoint must also build its archive on a cold cache.
        clear_convoy_cache(&app.dirs().spt_server);
    }
}

async fn assert_archives_fail(app: &mut TestApp, id: i64) {
    for request in archive_requests(id) {
        let uri = request.uri().to_string();
        let response = app.send_raw(request).await;
        assert_eq!(
            response.status(),
            StatusCode::INTERNAL_SERVER_ERROR,
            "{uri}"
        );
    }
}

#[actix_web::test]
async fn convoy_archives_require_explicit_enablement_even_when_cached() {
    let mut app = TestAppBuilder::new().with_nested_layout().build().await;
    let id = seed_client_mod(&app);
    write_file(&app.dirs().mod_overlay(), RECEIPT, CONTENT);
    let warm = app.send_raw(batch_request(id, false)).await;
    assert_eq!(warm.status(), StatusCode::OK);

    for config in [
        None,
        Some(ConvoyConfig {
            enabled: false,
            ..ConvoyConfig::default()
        }),
    ] {
        app.set_convoy_config(config);
        for request in archive_requests(id) {
            let uri = request.uri().to_string();
            let response = app.send_raw(request).await;
            assert_eq!(response.status(), StatusCode::SERVICE_UNAVAILABLE, "{uri}");
        }
    }
}

#[actix_web::test]
async fn convoy_archives_include_overlay_only_client_file() {
    let mut app = TestAppBuilder::new().with_nested_layout().build().await;
    let id = seed_client_mod(&app);
    write_file(&app.dirs().mod_overlay(), RECEIPT, CONTENT);
    assert!(!app.dirs().spt_server.join(RECEIPT).exists());
    assert!(!app.dirs().spt_runtime().exists());

    assert_archives_match_catalog(&mut app, id).await;
}

#[actix_web::test]
async fn convoy_archives_include_direct_storage_client_file() {
    let mut app = TestAppBuilder::new().build().await;
    let id = seed_client_mod(&app);
    write_file(&app.dirs().spt_server, RECEIPT, CONTENT);

    assert_archives_match_catalog(&mut app, id).await;
}

#[actix_web::test]
async fn convoy_archives_prefer_runtime_upper() {
    let mut app = TestAppBuilder::new().with_nested_layout().build().await;
    let id = seed_client_mod(&app);
    write_file(&app.dirs().spt_server, RECEIPT, b"base");
    write_file(&app.dirs().mod_overlay(), RECEIPT, b"mod overlay");
    write_file(&app.dirs().runtime_upper(), RECEIPT, CONTENT);

    assert_archives_match_catalog(&mut app, id).await;
}

#[actix_web::test]
async fn convoy_archives_fail_without_caching_partial_zip_when_catalog_file_is_missing() {
    let mut app = TestAppBuilder::new().with_nested_layout().build().await;
    let id = seed_client_mod(&app);
    write_file(&app.dirs().mod_overlay(), RECEIPT, CONTENT);
    app.db
        .lock()
        .insert_file(id, MISSING, Some(&checksum(CONTENT)), None)
        .unwrap();

    assert_archives_fail(&mut app, id).await;
    let cache = app.dirs().spt_server.join("quartermaster-cache/convoy");
    assert!(std::fs::read_dir(cache).unwrap().next().is_none());

    let response = app.send_raw(batch_request(id, true)).await;
    assert_eq!(response.status(), StatusCode::OK);
    let body = test::read_body(response).await;
    assert!(ZipArchive::new(Cursor::new(body)).unwrap().is_empty());
}

#[actix_web::test]
async fn convoy_archives_do_not_reveal_lower_layer_file_hidden_by_whiteout() {
    let mut app = TestAppBuilder::new().with_nested_layout().build().await;
    let id = seed_client_mod(&app);
    write_file(&app.dirs().spt_server, RECEIPT, CONTENT);
    write_file(
        &app.dirs().runtime_upper(),
        "BepInEx/plugins/ConvoySmokeFixture/.wh.receipt.txt",
        b"",
    );

    assert_archives_fail(&mut app, id).await;
}

#[cfg(target_os = "linux")]
#[actix_web::test]
async fn convoy_archives_do_not_fall_back_after_a_denied_client_read() {
    let mut app = TestAppBuilder::new().with_nested_layout().build().await;
    let id = seed_client_mod(&app);
    write_file(&app.dirs().spt_server, RECEIPT, CONTENT);
    let upper = app.dirs().runtime_upper();
    let server_path = "SPT_Runtime/user/profiles/fixture.json";
    write_file(&upper, server_path, b"server fixture");
    let receipt = upper.join(RECEIPT);
    std::fs::create_dir_all(receipt.parent().unwrap()).unwrap();
    std::os::unix::fs::symlink(upper.join(server_path), receipt).unwrap();

    assert_archives_fail(&mut app, id).await;
}

#[actix_web::test]
async fn convoy_archives_ignore_fresh_cache_from_before_layered_downloads() {
    let mut app = TestAppBuilder::new().build().await;
    let id = seed_client_mod(&app);
    write_file(&app.dirs().spt_server, RECEIPT, CONTENT);

    let mut hasher = DefaultHasher::new();
    [id].as_slice().hash(&mut hasher);
    false.hash(&mut hasher);
    let cache = app.dirs().spt_server.join("quartermaster-cache/convoy");
    std::fs::create_dir_all(&cache).unwrap();
    let old_path = cache.join(format!("convoy-{:016x}.zip", hasher.finish()));
    ZipWriter::new(std::fs::File::create(old_path).unwrap())
        .finish()
        .unwrap();

    assert_archives_match_catalog(&mut app, id).await;
}

#[actix_web::test]
async fn checksumless_eligible_files_reject_catalog() {
    for path in [
        "EscapeFromTarkov_Data/Managed/Required.dll",
        "BepInEx/plugins/Required.dll",
    ] {
        let app = TestAppBuilder::new().with_nested_layout().build().await;
        let id = seed_client_mod(&app);
        write_file(&app.dirs().mod_overlay(), path, CONTENT);
        app.db.lock().insert_file(id, path, None, None).unwrap();
        let error = generate_catalog(
            &app.db.lock(),
            &app.dirs().spt_server,
            &ConvoyConfig::default(),
        )
        .expect_err("incomplete packages must not advertise success");
        assert!(error.to_string().contains("checksum"), "{error}");
        assert!(error.to_string().contains(path), "{error}");
    }
}

#[actix_web::test]
async fn checksumless_eligible_files_reject_both_routes_cold() {
    checksumless_routes(false).await;
}

#[actix_web::test]
async fn checksumless_eligible_files_reject_both_routes_warm() {
    checksumless_routes(true).await;
}

async fn checksumless_routes(warm: bool) {
    for path in [
        "EscapeFromTarkov_Data/Managed/Required.dll",
        "BepInEx/plugins/Required.dll",
    ] {
        for route in 0..2 {
            let mut app = TestAppBuilder::new().with_nested_layout().build().await;
            let id = seed_client_mod(&app);
            write_file(&app.dirs().mod_overlay(), RECEIPT, CONTENT);
            write_file(&app.dirs().mod_overlay(), path, CONTENT);
            app.db.lock().insert_file(id, path, None, None).unwrap();
            clear_convoy_cache(&app.dirs().spt_server);
            if warm {
                // Seed the exact current cache identity with a previously built incomplete ZIP.
                // This must never bypass live inventory validity checks.
                let files = app.db.lock().get_files_for_mod_ids(&[id]).unwrap();
                let mut identity = DefaultHasher::new();
                let mut entries: Vec<_> = files
                    .iter()
                    .map(|f| (&f.file_path, &f.file_hash, &f.source))
                    .collect();
                entries.sort();
                entries.hash(&mut identity);
                let mut key = DefaultHasher::new();
                vec![id].hash(&mut key);
                false.hash(&mut key);
                let cache = app.dirs().spt_server.join("quartermaster-cache/convoy");
                std::fs::create_dir_all(&cache).unwrap();
                let file = std::fs::File::create(cache.join(format!(
                    "convoy-v3-{:016x}.zip-{:016x}.zip",
                    key.finish(),
                    identity.finish()
                )))
                .unwrap();
                let mut zip = ZipWriter::new(file);
                zip.start_file(path, zip::write::SimpleFileOptions::default())
                    .unwrap();
                std::io::Write::write_all(&mut zip, CONTENT).unwrap();
                zip.finish().unwrap();
            }
            let response = app
                .send_raw(archive_requests(id).into_iter().nth(route).unwrap())
                .await;
            assert_eq!(
                response.status(),
                StatusCode::INTERNAL_SERVER_ERROR,
                "path={path}, warm={warm}, route={route}"
            );
            let body = test::read_body(response).await;
            assert!(
                String::from_utf8_lossy(&body).contains("checksum"),
                "{}",
                String::from_utf8_lossy(&body)
            );
        }
    }
}

#[actix_web::test]
async fn managed_dependency_publisher_delivery_setup_and_cache() {
    let mut app = TestAppBuilder::new().with_nested_layout().build().await;
    let id = seed_client_mod(&app);
    let managed = "EscapeFromTarkov_Data/Managed/Unity.VectorGraphics.dll";
    let bridge = "EscapeFromTarkov_Data/Managed/Unity.InternalAPIEngineBridge.003.dll";
    let files = [RECEIPT, managed, bridge];
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
    let catalog = generate_catalog(
        &app.db.lock(),
        &app.dirs().spt_server,
        &ConvoyConfig::default(),
    )
    .unwrap();
    assert_eq!(catalog.groups[0].mods[0].file_checksums.len(), 3);
    let export = std::env::var_os("CONVOY_FIXTURE_EXPORT").map(std::path::PathBuf::from);
    if let Some(ref export) = export {
        std::fs::create_dir_all(export).unwrap();
        std::fs::write(
            export.join("catalog.json"),
            serde_json::to_vec(&catalog).unwrap(),
        )
        .unwrap();
    }
    for index in 0..2 {
        clear_convoy_cache(&app.dirs().spt_server);
        for warm in [false, true] {
            let request = archive_requests(id).into_iter().nth(index).unwrap();
            let response = app.send_raw(request).await;
            assert_eq!(
                response.status(),
                StatusCode::OK,
                "route={index}, warm={warm}"
            );
            let body = test::read_body(response).await;
            let mut zip = ZipArchive::new(Cursor::new(&body)).unwrap();
            assert_eq!(zip.len(), 3);
            for path in files {
                let mut got = Vec::new();
                zip.by_name(path).unwrap().read_to_end(&mut got).unwrap();
                assert_eq!(got, CONTENT);
            }
            if let Some(ref export) = export {
                std::fs::write(export.join(format!("archive-{index}.zip")), &body).unwrap();
            }
        }
    }
    if let Some(executable) = std::env::var_os("CONVOY_MONO_REGRESSION") {
        let state = app.app_state.clone();
        state.catalog_cache.invalidate();
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
        while state.catalog_cache.get().is_none() {
            assert!(
                std::time::Instant::now() < deadline,
                "catalog rebuild timed out"
            );
            tokio::time::sleep(std::time::Duration::from_millis(10)).await;
        }
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap();
        let server = actix_web::HttpServer::new(move || {
            actix_web::App::new()
                .app_data(state.clone())
                .route(
                    "/quma/convoy/catalog",
                    actix_web::web::get().to(spt_quartermaster::web::handlers::convoy::catalog),
                )
                .route(
                    "/quma/convoy/download",
                    actix_web::web::post().to(spt_quartermaster::web::handlers::convoy::download),
                )
                .route(
                    "/quma/convoy/mod/{mod_id}/archive",
                    actix_web::web::get()
                        .to(spt_quartermaster::web::handlers::convoy::single_mod_archive),
                )
                .route(
                    "/quma/convoy/report",
                    actix_web::web::post().to(spt_quartermaster::web::handlers::convoy::report),
                )
        })
        .workers(1)
        .listen(listener)
        .unwrap()
        .run();
        let handle = server.handle();
        actix_web::rt::spawn(server);
        let output = tokio::task::spawn_blocking(move || {
            std::process::Command::new("mono")
                .arg(executable)
                .arg("-config={\"BackendUrl\":\"http://127.0.0.1:1\",\"Version\":\"live\"}")
                .arg("-token=convoy-regression-fixture")
                .env("CONVOY_PUBLISHER_URL", format!("http://{address}"))
                .output()
                .unwrap()
        })
        .await
        .unwrap();
        handle.stop(true).await;
        if let Some(ref export) = export {
            std::fs::write(export.join("live-client.log"), &output.stdout).unwrap();
        }
        assert!(
            output.status.success(),
            "Mono consumer failed: {} {}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
    }
    // Identity must invalidate a warm archive even without an explicit cache clear.
    app.db
        .lock()
        .insert_file(id, MISSING, Some(&checksum(CONTENT)), None)
        .unwrap();
    assert_archives_fail(&mut app, id).await;
    write_file(&app.dirs().mod_overlay(), MISSING, CONTENT);
    let response = app.send_raw(batch_request(id, false)).await;
    assert_eq!(response.status(), StatusCode::OK);
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
