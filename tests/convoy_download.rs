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
