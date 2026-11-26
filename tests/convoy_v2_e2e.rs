use actix_governor::GovernorConfigBuilder;
use actix_web::{dev::Service, web, App, HttpServer};
use parking_lot::Mutex;
#[allow(dead_code)]
mod common;
#[path = "common/convoy_v2.rs"]
mod publication;
use spt_quartermaster::{config::ConvoyConfig, web::configure_app};
use std::{
    collections::BTreeMap, fs, net::TcpListener, path::PathBuf, process::Stdio, sync::Arc,
    time::Duration,
};

const ITEM: &str = "BepInEx/plugins/Test/item.dll";
const DEPENDENCY: &str = "EscapeFromTarkov_Data/Managed/Dependency.dll";
const CHILD_ARGS: &str = "-config={\"BackendUrl\":\"http://127.0.0.1:1\",\"Version\":\"live\"}";

const ABC128: &str = "06b05ab6733a618578af5f94892f3950";
const EMPTY128: &str = "99aa06d3014798d86001c324468d497f";

fn assert_artifacts(stdout: &str, dll: &std::path::Path, exe: &std::path::Path) {
    for (label, artifact) in [
        ("ASSEMBLY", dll.to_path_buf()),
        ("HASHING", dll.with_file_name("System.IO.Hashing.dll")),
        ("HARNESS", exe.to_path_buf()),
    ] {
        assert!(
            stdout
                .lines()
                .any(|line| line == format!("{label} {}", artifact.display())),
            "expected loaded {label} at {artifact:?} in {stdout}"
        );
    }
}

#[actix_web::test]
async fn convoy_v2_e2e_fresh_and_existing() {
    let required = std::env::var("CONVOY_REQUIRE_LIVE").as_deref() == Ok("1");
    let Some(exe) = std::env::var_os("CONVOY_MONO_REGRESSION").map(PathBuf::from) else {
        assert!(
            !required,
            "CONVOY_REQUIRE_LIVE=1 requires CONVOY_MONO_REGRESSION"
        );
        println!("SKIP Mono bridge: CONVOY_MONO_REGRESSION is not set");
        return;
    };
    assert!(
        exe.is_absolute() && exe.is_file(),
        "invalid Mono executable: {exe:?}"
    );
    let mono_path = std::env::var_os("MONO_PATH")
        .map(PathBuf::from)
        .expect("set MONO_PATH to the Convoy release output");
    let dll = mono_path.join("Convoy.dll");
    assert!(
        mono_path.is_absolute() && dll.is_file(),
        "invalid Convoy assembly: {dll:?}"
    );

    let app = common::TestAppBuilder::new().build().await;
    let dirs = app.dirs();
    let id = app
        .db
        .lock()
        .insert_mod(None, None, "Mono bridge", None, "1", "local", None, None)
        .unwrap();
    for (relative, bytes, xxh) in [
        (ITEM, &b"abc"[..], ABC128),
        (DEPENDENCY, &b""[..], EMPTY128),
    ] {
        let path = dirs.spt_server.join(relative);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, bytes).unwrap();
        app.db
            .lock()
            .insert_file(id, relative, Some(xxh), Some(bytes.len() as i64))
            .unwrap();
    }
    app.set_convoy_config(Some(ConvoyConfig {
        enabled: true,
        max_store_bytes: 3,
        ..Default::default()
    }));
    let state = app.app_state.clone();
    publication::ready(&state.convoy_publisher).await;
    let key = app.session_key.clone();
    let governor = GovernorConfigBuilder::default().finish().unwrap();
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let addr = listener.local_addr().unwrap();
    let origin = format!("http://{addr}");
    let requests = Arc::new(Mutex::new(Vec::new()));
    let access_log = requests.clone();
    let server = HttpServer::new(move || {
        let access_log = access_log.clone();
        App::new()
            .app_data(state.clone())
            .app_data(state.convoy_publisher.clone())
            .wrap_fn(move |req, svc| {
                access_log.lock().push((
                    req.method().to_string(),
                    req.path().to_owned(),
                    req.headers().contains_key("range"),
                ));
                svc.call(req)
            })
            .configure(|cfg| {
                configure_app(
                    cfg,
                    key.clone(),
                    false,
                    true,
                    Some(governor.clone()),
                    Some(governor.clone()),
                )
            })
    })
    .workers(1)
    .disable_signals()
    .listen(listener)
    .unwrap()
    .run();
    let handle = server.handle();
    let task = actix_web::rt::spawn(server);

    // Defer assertions until both children and the listener have been cleaned up.
    let mut results = Vec::new();
    for (name, matching) in [
        ("live Rust publisher fresh", false),
        ("live Rust publisher matching", true),
    ] {
        let output = tokio::time::timeout(
            Duration::from_secs(45),
            tokio::process::Command::new("/usr/bin/time")
                .args(["-f", "child_elapsed_s=%e peak_rss_kib=%M", "mono"])
                .arg(&exe)
                .arg("-config={\"BackendUrl\":\"http://127.0.0.1:1\",\"Version\":\"live\"}")
                .arg("-token=convoy-regression-fixture")
                .env("CONVOY_PUBLISHER_URL", &origin)
                .env("CONVOY_CASE", name)
                .stdout(Stdio::piped())
                .stderr(Stdio::piped())
                .kill_on_drop(true)
                .output(),
        )
        .await;
        let captured = std::mem::take(&mut *requests.lock());
        results.push((name, matching, output, captured));
    }
    let http = reqwest::Client::new();
    for enabled in [true, false] {
        app.set_convoy_config(Some(ConvoyConfig {
            enabled,
            ..Default::default()
        }));
        for (method, path) in [
            (reqwest::Method::GET, "/quma/convoy/catalog"),
            (reqwest::Method::POST, "/quma/convoy/download"),
            (reqwest::Method::GET, "/quma/convoy/mod/1/archive"),
        ] {
            let response = http
                .request(method, format!("{origin}{path}"))
                .send()
                .await
                .unwrap();
            assert_eq!(response.status().as_u16(), if enabled { 410 } else { 503 });
        }
    }
    let disabled = http
        .get(format!("{origin}/quma/convoy/v3/catalog"))
        .send()
        .await
        .unwrap();
    assert_eq!(disabled.status(), 503);
    assert_eq!(
        disabled.json::<serde_json::Value>().await.unwrap()["code"],
        "disabled"
    );
    let reports = app.db.lock().get_latest_sync_reports().unwrap();
    assert_eq!(reports.len(), 1);
    assert_eq!(reports[0].result, "up_to_date");
    handle.stop(true).await;
    task.await.unwrap().unwrap();
    assert!(
        std::net::TcpStream::connect_timeout(&addr, Duration::from_secs(1)).is_err(),
        "listener survived test teardown"
    );

    for (name, matching, output, requests) in results {
        let output = output
            .expect("Mono regression timed out")
            .expect("failed to run Mono");
        let stdout = String::from_utf8(output.stdout).unwrap();
        let stderr = String::from_utf8_lossy(&output.stderr);
        println!("{name}: {}\n{stdout}{stderr}", output.status);
        assert!(output.status.success(), "Mono consumer failed: {name}");
        assert!(stdout.lines().any(|line| line == format!("PASS: {name}")));
        assert_artifacts(&stdout, &dll, &exe);
        let owned = if matching { "False" } else { "True" };
        for (path, len, xxh) in [(ITEM, 3, ABC128), (DEPENDENCY, 0, EMPTY128)] {
            assert!(stdout.lines().any(
                |line| line == format!("LIVE {path} length={len} xxh3_128={xxh} owned={owned}")
            ));
        }
        let mut objects = BTreeMap::from([(ABC128, 0), (EMPTY128, 0)]);
        let mut catalogs = 0;
        let mut reports = 0;
        for (method, path, range) in &requests {
            assert!(!range, "Task 2 must use full GETs");
            match (method.as_str(), path.as_str()) {
                ("GET", "/quma/convoy/v3/catalog") => catalogs += 1,
                ("POST", "/quma/convoy/report") => reports += 1,
                ("GET", path)
                    if path.starts_with("/quma/convoy/v3/generations/")
                        && path.contains("/objects/") =>
                {
                    let digest = path.rsplit_once('/').unwrap().1;
                    *objects.get_mut(digest).expect("unknown requested object") += 1;
                }
                _ => panic!("unexpected or legacy transport: {method} {path}"),
            }
        }
        println!("{name}: catalogs={catalogs} reports={reports} object GETs={objects:?}");
        assert_eq!((catalogs, reports), (4, 2));
        assert_eq!(
            objects,
            BTreeMap::from([
                (ABC128, i32::from(!matching)),
                (EMPTY128, i32::from(!matching))
            ])
        );
    }
}

#[actix_web::test]
async fn convoy_v2_e2e_source_republish_fences_owned_update() {
    use spt_quartermaster::convoy::publisher::PublishMode;
    use std::sync::atomic::{AtomicBool, Ordering};

    let Some(exe) = std::env::var_os("CONVOY_MONO_REGRESSION").map(PathBuf::from) else {
        assert_ne!(std::env::var("CONVOY_REQUIRE_LIVE").as_deref(), Ok("1"));
        println!("SKIP source republish bridge: no CONVOY_MONO_REGRESSION");
        return;
    };
    let dll = PathBuf::from(std::env::var_os("MONO_PATH").unwrap()).join("Convoy.dll");
    // Negative control changes the same source bytes at the same point but deliberately
    // leaves the committed generation alone. The production path should then apply.
    let republish = std::env::var("CONVOY_TEST_OMIT_SOURCE_REPUBLISH").as_deref() != Ok("1");
    const OLD: &[u8] = b"known owned original";
    const STAGED: &[u8] = b"replacement verified by Convoy";
    const DRIFT: &[u8] = b"different tracked source bytes after staging and before apply";
    const NEW_PATH: &str = "BepInEx/plugins/Test/also-new.dll";
    const NEW_BYTES: &[u8] = b"new target must not be applied";
    const CATALOG: &str = "/quma/convoy/v3/catalog";
    let digest = |bytes: &[u8]| format!("{:032x}", xxhash_rust::xxh3::xxh3_128(bytes));

    let app = common::TestAppBuilder::new().build().await;
    let game = tempfile::tempdir().unwrap();
    let root = game.path();
    let state_path = root.join("BepInEx/config/Convoy/state.json");
    let source = app.dirs().spt_server.join(ITEM);
    fs::create_dir_all(source.parent().unwrap()).unwrap();
    fs::write(&source, OLD).unwrap();
    let id = {
        let db = app.db.lock();
        let id = db
            .insert_mod(
                None,
                None,
                "Source republish bridge",
                None,
                "1",
                "local",
                None,
                None,
            )
            .unwrap();
        db.insert_file(id, ITEM, Some(&digest(OLD)), Some(OLD.len() as i64))
            .unwrap();
        id
    };
    app.set_convoy_config(Some(ConvoyConfig {
        enabled: true,
        ..Default::default()
    }));
    let publisher = app.app_state.convoy_publisher.clone();
    publisher.refresh(PublishMode::Managed);
    let original_generation = publication::ready(&publisher).await;

    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let origin = format!("http://{}", listener.local_addr().unwrap());
    let armed = Arc::new(AtomicBool::new(false));
    let evidence = Arc::new(Mutex::new(None));
    let server = {
        let armed = armed.clone();
        let evidence = evidence.clone();
        let state = app.app_state.clone();
        let key = app.session_key.clone();
        let root = root.to_owned();
        let source = source.clone();
        let governor = GovernorConfigBuilder::default().finish().unwrap();
        HttpServer::new(move || {
            let (root, source, armed, evidence, publisher) = (root.clone(), source.clone(), armed.clone(), evidence.clone(), state.convoy_publisher.clone());
            App::new().app_data(state.clone()).app_data(publisher.clone())
                .wrap_fn(move |req, svc| {
                    let validator = req.headers().get("If-None-Match")
                        .filter(|_| req.path() == CATALOG && armed.swap(false, Ordering::SeqCst))
                        .map(|h| h.to_str().unwrap().to_owned());
                    let (root, source, evidence, publisher) = (root.clone(), source.clone(), evidence.clone(), publisher.clone());
                    let response = svc.call(req);
                    async move {
                        if let Some(validator) = validator {
                            let spool = root.join("BepInEx/config/Convoy/transfer-v2");
                            for bytes in [STAGED, NEW_BYTES] {
                                let sha = digest(bytes);
                                let suffix = format!("{sha}-{}.data", bytes.len());
                                let checkpoint = fs::read_dir(&spool).unwrap().map(|e| e.unwrap().path())
                                    .find(|p| p.file_name().unwrap().to_string_lossy().ends_with(&suffix)).expect("mapped verified staging object");
                                assert_eq!(fs::read(&checkpoint).unwrap(), bytes);
                                assert_eq!(fs::read_to_string(checkpoint.with_extension("meta")).unwrap(), format!("\"{sha}\"\n{}", bytes.len()));
                                println!("verified checkpoint at conditional: {sha}, {} bytes", bytes.len());
                            }
                            assert_eq!(fs::read(root.join(ITEM)).unwrap(), OLD);
                            assert!(!root.join(NEW_PATH).exists());
                            let start = std::time::Instant::now();
                            assert_eq!(fs::read(&source).unwrap(), STAGED);
                            fs::write(&source, DRIFT).unwrap();
                            let generation = if republish {
                                publisher.refresh(PublishMode::Rehash);
                                Some(publication::ready(&publisher).await)
                            } else { None };
                            let response = response.await?;
                            let etag = response.headers().get("ETag").map(|h| h.to_str().unwrap().to_owned());
                            println!("source drift republish={republish}: old If-None-Match={validator}, new Ready={generation:?}, HTTP {}, ETag={etag:?}, {:?}", response.status(), start.elapsed());
                            *evidence.lock() = Some((validator, generation, response.status().as_u16(), etag));
                            Ok(response)
                        } else { response.await }
                    }
                })
                .configure(|cfg| configure_app(cfg, key.clone(), false, true, Some(governor.clone()), Some(governor.clone())))
        }).workers(1).disable_signals().listen(listener).unwrap().run()
    };
    let handle = server.handle();
    let task = actix_web::rt::spawn(server);
    let run = || {
        let mut command = tokio::process::Command::new("mono");
        command
            .arg(&exe)
            .arg(CHILD_ARGS)
            .arg("-token=convoy...re")
            .env("CONVOY_CHILD_ROOT", root)
            .env("CONVOY_CHILD_URL", &origin)
            .kill_on_drop(true);
        tokio::time::timeout(Duration::from_secs(45), command.output())
    };
    let first = run().await.unwrap().unwrap();
    assert_artifacts(&String::from_utf8_lossy(&first.stdout), &dll, &exe);
    assert!(first.status.success(), "{:?}", first);
    let baseline = fs::read(&state_path).unwrap();
    let state: serde_json::Value = serde_json::from_slice(&baseline).unwrap();
    assert_eq!(state["server_url"], origin);
    assert_eq!(state["mods"][0]["id"], id);
    assert_eq!(state["mods"][0]["files"][0]["path"], ITEM);
    assert_eq!(state["mods"][0]["files"][0]["owned"], true);
    assert_eq!(state["mods"][0]["files"][0]["length"], OLD.len());
    assert_eq!(state["mods"][0]["files"][0]["hash"], digest(OLD));
    assert!(state["mods"][0]["files"][0].get("sha256").is_none());
    assert_eq!(fs::read(root.join(ITEM)).unwrap(), OLD);

    fs::write(&source, STAGED).unwrap();
    fs::write(app.dirs().spt_server.join(NEW_PATH), NEW_BYTES).unwrap();
    app.db
        .lock()
        .insert_file(
            id,
            NEW_PATH,
            Some(&digest(NEW_BYTES)),
            Some(NEW_BYTES.len() as i64),
        )
        .unwrap();
    publisher.refresh(PublishMode::Rehash);
    let staged_generation = publication::ready(&publisher).await;
    assert_ne!(original_generation, staged_generation);
    assert!(!root.join(NEW_PATH).exists());
    armed.store(true, Ordering::SeqCst);
    let second = run().await.unwrap().unwrap();
    let stdout = String::from_utf8_lossy(&second.stdout);
    println!(
        "source fence child {}\n{stdout}{}",
        second.status,
        String::from_utf8_lossy(&second.stderr)
    );
    assert_artifacts(&stdout, &dll, &exe);
    let (validator, generation, status, etag) =
        evidence.lock().take().expect("real final conditional");
    assert_eq!(validator, format!("\"{staged_generation}\""));
    if let Some(generation) = generation {
        assert_ne!(generation, staged_generation);
        assert_eq!(status, 200);
        assert_eq!(etag.as_deref(), Some(format!("\"{generation}\"").as_str()));
        let http = reqwest::Client::new();
        let catalog = http
            .get(format!("{origin}{CATALOG}"))
            .send()
            .await
            .unwrap()
            .error_for_status()
            .unwrap()
            .json::<serde_json::Value>()
            .await
            .unwrap();
        assert_eq!(catalog["generation"], generation);
        let files = catalog["groups"][0]["mods"][0]["files"].as_array().unwrap();
        let changed = files.iter().find(|file| file["path"] == ITEM).unwrap();
        assert_eq!(changed["xxh3_128"], digest(DRIFT));
        assert_eq!(changed["length"], DRIFT.len());
        let bytes = http
            .get(format!(
                "{origin}/quma/convoy/v3/generations/{generation}/objects/{}",
                digest(DRIFT)
            ))
            .send()
            .await
            .unwrap()
            .error_for_status()
            .unwrap()
            .bytes()
            .await
            .unwrap();
        assert_eq!(bytes.as_ref(), DRIFT);
        println!(
            "real republished object: {} bytes, xxh3_128={}",
            DRIFT.len(),
            digest(DRIFT)
        );
    } else {
        assert_eq!(status, 304);
        assert_eq!(fs::read(root.join(ITEM)).unwrap(), STAGED);
        assert_eq!(fs::read(root.join(NEW_PATH)).unwrap(), NEW_BYTES);
        assert_ne!(fs::read(&state_path).unwrap(), baseline);
        println!("negative circuit applied staged content without a committed republish");
    }
    handle.stop(true).await;
    task.await.unwrap().unwrap();
    assert_eq!(
        second.status.code(),
        Some(15),
        "expected Replan after a committed source republish"
    );
    assert!(stdout.lines().any(|line| line.ends_with(": Replan")));
    assert_eq!(fs::read(&state_path).unwrap(), baseline);
    assert_eq!(fs::read(root.join(ITEM)).unwrap(), OLD);
    assert!(!root.join(NEW_PATH).exists());
}

const LARGE_TOTAL: u64 = 3 * 1024 * 1024 * 1024;
const SMALL_LEN: u64 = 1024 * 1024;
const BIG_LEN: u64 = LARGE_TOTAL - SMALL_LEN;
const CUT: u64 = 8 * SMALL_LEN;

#[derive(Debug)]
struct Wire {
    phase: u8,
    digest: String,
    range: Option<String>,
    if_range: Option<String>,
    status: u16,
    declared: u64,
    emitted: u64,
    first_ms: Option<u128>,
}

fn fixture_file(path: &std::path::Path, size: u64, salt: u8) -> (String, String) {
    use std::io::Write;
    let mut file = fs::File::create(path).unwrap();
    let mut xxh = xxhash_rust::xxh3::Xxh3::new();
    let mut chunk = vec![salt; SMALL_LEN as usize];
    for offset in (0..size).step_by(chunk.len()) {
        chunk[..8].copy_from_slice(&offset.to_le_bytes());
        let bytes = &chunk[..(size - offset).min(SMALL_LEN) as usize];
        file.write_all(bytes).unwrap();
        xxh.update(bytes);
    }
    file.sync_all().unwrap();
    (
        format!("{:032x}", xxh.digest128()),
        format!("{:032x}", xxh.digest128()),
    )
}

fn destination_hash(path: &std::path::Path) -> String {
    use std::io::Read;
    let mut file = fs::File::open(path).unwrap();
    let mut block = vec![0; SMALL_LEN as usize];
    let mut xxh = xxhash_rust::xxh3::Xxh3::new();
    loop {
        let n = file.read(&mut block).unwrap();
        if n == 0 {
            break;
        }
        xxh.update(&block[..n]);
    }
    format!("{:032x}", xxh.digest128())
}

struct Activity(Arc<Mutex<(usize, usize)>>);
impl Activity {
    fn begin(counters: Arc<Mutex<(usize, usize)>>) -> Self {
        {
            let mut c = counters.lock();
            c.0 += 1;
            c.1 = c.1.max(c.0);
        }
        Self(counters)
    }
}
impl Drop for Activity {
    fn drop(&mut self) {
        self.0.lock().0 -= 1;
    }
}

#[actix_web::test]
async fn convoy_v2_e2e_large_resume() {
    use actix_web::body::{BodySize, MessageBody, SizedStream};
    use spt_quartermaster::convoy::publisher::PublicationStatus;
    use std::{
        sync::atomic::{AtomicU8, Ordering},
        time::Instant,
    };
    if std::env::var("CONVOY_V2_LARGE").as_deref() != Ok("1") {
        println!("SKIP measured large bridge: set CONVOY_V2_LARGE=1");
        return;
    }
    let exe = PathBuf::from(
        std::env::var_os("CONVOY_MONO_REGRESSION").expect("Mono regression executable"),
    );
    let dll = PathBuf::from(std::env::var_os("MONO_PATH").expect("Mono plugin directory"))
        .join("Convoy.dll");
    let app = common::TestAppBuilder::new()
        .with_proxy_auth()
        .build()
        .await;
    app.set_proxy_auth(false);
    let upstream_socket = TcpListener::bind("127.0.0.1:0").unwrap();
    app.app_state.config.write().server_port = Some(upstream_socket.local_addr().unwrap().port());
    let cert = rcgen::generate_simple_self_signed(vec!["localhost".into()]).unwrap();
    let tls = rustls::ServerConfig::builder()
        .with_no_client_auth()
        .with_single_cert(
            vec![cert.cert.der().clone()],
            rustls::pki_types::PrivatePkcs8KeyDer::from(cert.signing_key.serialize_der()).into(),
        )
        .unwrap();
    let upstream = HttpServer::new(|| {
        App::new().route(
            "/launcher/v2/ping",
            web::get().to(|| async { "fixture upstream ready" }),
        )
    })
    .workers(1)
    .disable_signals()
    .listen_rustls_0_23(upstream_socket, tls)
    .unwrap()
    .run();
    let upstream_handle = upstream.handle();
    let upstream_task = actix_web::rt::spawn(upstream);
    let game = tempfile::tempdir().unwrap();
    let game_root = game.path().to_path_buf();
    let spool = game_root.join("BepInEx/config/Convoy/transfer-v2");
    let baseline = game_root.join("BepInEx/config/Convoy/state.json");
    fs::create_dir_all(baseline.parent().unwrap()).unwrap();
    fs::write(&baseline, b"{}").unwrap();
    let sentinel = game_root.join("local.txt");
    fs::write(&sentinel, b"untouched locally").unwrap();
    let id = app
        .db
        .lock()
        .insert_mod(
            None,
            None,
            "Large loopback bridge",
            None,
            "1",
            "local",
            None,
            None,
        )
        .unwrap();
    let large_path = "BepInEx/plugins/Test/Large.dll";
    let small_path = "BepInEx/plugins/Test/Small.dll";
    let source = app.dirs().spt_server.clone();
    let write_start = Instant::now();
    let (big, small) = tokio::task::spawn_blocking(move || {
        fs::create_dir_all(source.join("BepInEx/plugins/Test")).unwrap();
        (
            fixture_file(&source.join(large_path), BIG_LEN, 0x71),
            fixture_file(&source.join(small_path), SMALL_LEN, 0xb2),
        )
    })
    .await
    .unwrap();
    println!("fixture write/hash {:?}: large={BIG_LEN} {big:?}, small={SMALL_LEN} {small:?}, total={LARGE_TOTAL}", write_start.elapsed());
    for (path, length, identity) in [(large_path, BIG_LEN, &big), (small_path, SMALL_LEN, &small)] {
        app.db
            .lock()
            .insert_file(id, path, Some(&identity.1), Some(length as i64))
            .unwrap();
    }

    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let origin = format!("http://{}", listener.local_addr().unwrap());
    let state = app.app_state.clone();
    let key = app.session_key.clone();
    let governor = GovernorConfigBuilder::default().finish().unwrap();
    let fence = Arc::new(std::sync::atomic::AtomicBool::new(false));
    let phase = Arc::new(AtomicU8::new(0));
    let log = Arc::new(Mutex::new(Vec::<Wire>::new()));
    let bodies = Arc::new(Mutex::new((0, 0)));
    let server = {
        let (phase, log, big, small, bodies) = (
            phase.clone(),
            log.clone(),
            big.0.clone(),
            small.0.clone(),
            bodies.clone(),
        );
        let fence = fence.clone();
        HttpServer::new(move || {
            let fence = fence.clone();
            let db = state.db.clone();
            let (phase, log, big, small, spool, bodies) = (
                phase.clone(),
                log.clone(),
                big.clone(),
                small.clone(),
                spool.clone(),
                bodies.clone(),
            );
            App::new()
                .app_data(state.clone())
                .app_data(state.convoy_publisher.clone())
                .wrap_fn(move |req, svc| {
                    let digest = req
                        .path()
                        .split_once("/objects/")
                        .map(|(_, d)| d.to_owned());
                    let range = req
                        .headers()
                        .get("Range")
                        .map(|h| h.to_str().unwrap().to_owned());
                    let if_range = req
                        .headers()
                        .get("If-Range")
                        .map(|h| h.to_str().expect("ASCII strong validator").to_owned());
                    let stage = phase.load(Ordering::SeqCst);
                    let started = Instant::now();
                    let (log, big, small, spool, bodies) = (
                        log.clone(),
                        big.clone(),
                        small.clone(),
                        spool.clone(),
                        bodies.clone(),
                    );
                    if stage == 3
                        && req.path() == "/quma/convoy/v3/catalog"
                        && req.headers().contains_key("If-None-Match")
                        && !fence.swap(true, Ordering::SeqCst)
                    {
                        assert!(
                            log.lock()
                                .iter()
                                .filter(|w| w.phase == 3)
                                .map(|w| w.emitted)
                                .sum::<u64>()
                                == LARGE_TOTAL,
                            "revision changed before all bytes were staged"
                        );
                        db.lock()
                            .conn()
                            .execute("UPDATE installed_mods SET version = '2' WHERE id = ?", [id])
                            .unwrap();
                    }
                    let call = svc.call(req);
                    async move {
                        let response = call.await?;
                        Ok(response.map_body(move |head, body| {
                            let (Some(digest), BodySize::Sized(declared)) = (digest, body.size())
                            else {
                                return body;
                            };
                            let status = head.status.as_u16();
                            let cap = if stage == 1 && digest == big {
                                CUT
                            } else {
                                u64::MAX
                            };
                            let index = {
                                let mut items = log.lock();
                                let index = items.len();
                                items.push(Wire {
                                    phase: stage,
                                    digest,
                                    range,
                                    if_range,
                                    status,
                                    declared,
                                    emitted: 0,
                                    first_ms: None,
                                });
                                index
                            };
                            let stream = futures_util::stream::try_unfold(
                                (Box::pin(body), 0u64, Activity::begin(bodies)),
                                move |(mut body, sent, activity)| {
                                    let (log, spool, small) =
                                        (log.clone(), spool.clone(), small.clone());
                                    async move {
                                        if sent >= cap {
                                            return Err(std::io::Error::other(
                                                "intentional cut after forwarded bytes",
                                            ));
                                        }
                                        if cap != u64::MAX && sent == 0 {
                                            let deadline = Instant::now() + Duration::from_secs(10);
                                            while !fs::read_dir(&spool).is_ok_and(|mut entries| {
                                                entries.any(|e| {
                                                    e.is_ok_and(|e| {
                                                        e.file_name().to_string_lossy().ends_with(
                                                            &format!("{small}-{SMALL_LEN}.meta"),
                                                        )
                                                    })
                                                })
                                            }) {
                                                if Instant::now() > deadline {
                                                    return Err(std::io::Error::other(
                                                        "small checkpoint not persisted",
                                                    ));
                                                }
                                                tokio::time::sleep(Duration::from_millis(10)).await;
                                            }
                                        }
                                        match futures_util::future::poll_fn(|cx| {
                                            body.as_mut().poll_next(cx)
                                        })
                                        .await
                                        {
                                            Some(Ok(bytes)) => {
                                                let n = bytes.len() as u64;
                                                let mut items = log.lock();
                                                items[index].first_ms.get_or_insert_with(|| {
                                                    started.elapsed().as_millis()
                                                });
                                                items[index].emitted += n;
                                                Ok(Some((bytes, (body, sent + n, activity))))
                                            }
                                            Some(Err(err)) => {
                                                Err(std::io::Error::other(err.to_string()))
                                            }
                                            None => Ok(None),
                                        }
                                    }
                                },
                            );
                            SizedStream::new(declared, stream).boxed()
                        }))
                    }
                })
                .configure(|cfg| {
                    configure_app(
                        cfg,
                        key.clone(),
                        false,
                        true,
                        Some(governor.clone()),
                        Some(governor.clone()),
                    )
                })
        })
        .workers(1)
        .disable_signals()
        .listen(listener)
        .unwrap()
        .run()
    };
    let handle = server.handle();
    let service = actix_web::rt::spawn(server);
    let http = reqwest::Client::builder()
        .timeout(Duration::from_secs(4))
        .build()
        .unwrap();
    let publisher = &app.app_state.convoy_publisher;
    let publish_start = Instant::now();
    let mut status = publisher.subscribe();
    app.set_convoy_config(Some(ConvoyConfig {
        enabled: true,
        max_store_bytes: LARGE_TOTAL,
        ..Default::default()
    }));
    publisher.refresh(spt_quartermaster::convoy::publisher::PublishMode::Managed);
    assert_eq!(publisher.status(), PublicationStatus::Preparing);
    let proxy_start = Instant::now();
    let proxy = http
        .get(format!("{origin}/launcher/v2/ping"))
        .send()
        .await
        .unwrap()
        .status();
    assert!(proxy.is_success(), "real upstream proxy status: {proxy}");
    let db_probe = tokio::task::spawn_blocking({
        let db = app.db.clone();
        move || db.lock().get_latest_sync_reports().unwrap()
    });
    assert!(tokio::time::timeout(Duration::from_secs(2), db_probe)
        .await
        .unwrap()
        .unwrap()
        .is_empty());
    assert_eq!(publisher.status(), PublicationStatus::Preparing);
    println!(
        "publish-phase probe: proxy HTTP {proxy}, DB responsive in {:?}, status {:?}",
        proxy_start.elapsed(),
        publisher.status()
    );
    let ready = tokio::time::timeout(
        Duration::from_secs(360),
        status.wait_for(|s| {
            matches!(
                s,
                PublicationStatus::Ready { .. } | PublicationStatus::Failed { .. }
            )
        }),
    )
    .await
    .unwrap()
    .unwrap()
    .clone();
    assert!(
        matches!(
            ready,
            PublicationStatus::Ready {
                bytes: LARGE_TOTAL,
                ..
            }
        ),
        "{ready:?}"
    );
    println!("initial publication {:?}, {ready:?}; publisher limit 16, client limit 4, store cap {LARGE_TOTAL}", publish_start.elapsed());

    let PublicationStatus::Ready { ref generation, .. } = ready else {
        unreachable!()
    };
    let ttfb_start = Instant::now();
    let mut first_byte_probe = http
        .get(format!(
            "{origin}/quma/convoy/v3/generations/{generation}/objects/{}",
            big.0
        ))
        .send()
        .await
        .unwrap();
    assert_eq!(first_byte_probe.status(), 200);
    let first_chunk = first_byte_probe.chunk().await.unwrap().unwrap();
    println!(
        "loopback ready-object TTFB {:?} (first chunk {} bytes)",
        ttfb_start.elapsed(),
        first_chunk.len()
    );
    drop(first_byte_probe);

    let run = |root: &std::path::Path| {
        let mut command = tokio::process::Command::new("/usr/bin/time");
        command
            .args(["-f", "child_elapsed_s=%e peak_rss_kib=%M", "mono"])
            .arg(&exe)
            .arg(CHILD_ARGS)
            .arg("-token=convoy...re")
            .env("CONVOY_CHILD_ROOT", root)
            .env("CONVOY_CHILD_URL", &origin)
            .kill_on_drop(true);
        tokio::time::timeout(Duration::from_secs(480), command.output())
    };
    phase.store(1, Ordering::SeqCst);
    let first = run(&game_root).await.unwrap().unwrap();
    println!(
        "interrupted: {}\n{}{}",
        first.status,
        String::from_utf8_lossy(&first.stdout),
        String::from_utf8_lossy(&first.stderr)
    );
    assert_eq!(first.status.code(), Some(11));
    assert_artifacts(&String::from_utf8_lossy(&first.stdout), &dll, &exe);
    assert_eq!(fs::read(&baseline).unwrap(), b"{}");
    assert_eq!(fs::read(&sentinel).unwrap(), b"untouched locally");
    assert!(!game_root.join(large_path).exists() && !game_root.join(small_path).exists());
    let spool = game_root.join("BepInEx/config/Convoy/transfer-v2");
    let checkpoint = |digest: &str, len: u64| {
        let data = fs::read_dir(&spool)
            .unwrap()
            .map(|e| e.unwrap().path())
            .find(|p| {
                p.to_string_lossy()
                    .ends_with(&format!("{digest}-{len}.data"))
            })
            .expect("real checkpoint data");
        let saved = fs::metadata(&data).unwrap().len();
        assert_eq!(
            fs::read_to_string(data.with_extension("meta")).unwrap(),
            format!("\"{digest}\"\n{saved}")
        );
        saved
    };
    assert_eq!(checkpoint(&small.0, SMALL_LEN), SMALL_LEN);
    let saved = checkpoint(&big.0, BIG_LEN);
    println!("persisted resume offset {saved}");
    const INTERRUPTIONS: u64 = 4;
    assert_eq!(saved, INTERRUPTIONS * CUT);
    let validator = format!("\"{}\"", big.0);
    {
        let log = log.lock();
        let interrupted: Vec<_> = log
            .iter()
            .filter(|w| w.phase == 1 && w.digest == big.0)
            .collect();
        let smalls: Vec<_> = log
            .iter()
            .filter(|w| w.phase == 1 && w.digest == small.0)
            .collect();
        assert_eq!(interrupted.len() as u64, INTERRUPTIONS);
        assert_eq!(
            log.iter().filter(|w| w.phase == 1).count(),
            interrupted.len() + 1
        );
        assert_eq!(smalls.len(), 1);
        assert!(smalls[0].range.is_none() && smalls[0].if_range.is_none());
        assert_eq!(
            (smalls[0].status, smalls[0].declared, smalls[0].emitted),
            (200, SMALL_LEN, SMALL_LEN)
        );
        for (attempt, request) in interrupted.into_iter().enumerate() {
            let offset = attempt as u64 * CUT;
            assert_eq!(
                (request.status, request.declared, request.emitted),
                (if offset == 0 { 200 } else { 206 }, BIG_LEN - offset, CUT)
            );
            if offset == 0 {
                assert!(request.range.is_none() && request.if_range.is_none());
            } else {
                assert_eq!(
                    request.range.as_deref(),
                    Some(format!("bytes={offset}-").as_str())
                );
                assert_eq!(request.if_range.as_deref(), Some(validator.as_str()));
            }
        }
    }

    phase.store(2, Ordering::SeqCst);
    let resume = run(&game_root);
    let probe = async {
        let start = Instant::now();
        while bodies.lock().0 == 0 {
            assert!(
                start.elapsed() < Duration::from_secs(45),
                "no active resume body"
            );
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
        let start = Instant::now();
        let proxy = http
            .get(format!("{origin}/launcher/v2/ping"))
            .send()
            .await
            .unwrap()
            .status();
        assert!(proxy.is_success(), "real upstream proxy status: {proxy}");
        let db = app.db.clone();
        tokio::time::timeout(
            Duration::from_secs(3),
            tokio::task::spawn_blocking(move || db.lock().get_latest_sync_reports().unwrap()),
        )
        .await
        .unwrap()
        .unwrap();
        assert_eq!(publisher.status(), ready);
        assert!(
            bodies.lock().0 > 0,
            "body finished before responsiveness probe"
        );
        println!(
            "resume body probe: proxy HTTP {proxy}, DB responsive in {:?}",
            start.elapsed()
        );
    };
    let (resumed, ()) = tokio::join!(resume, probe);
    let resumed = resumed.unwrap().unwrap();
    println!(
        "resumed: {}\n{}{}",
        resumed.status,
        String::from_utf8_lossy(&resumed.stdout),
        String::from_utf8_lossy(&resumed.stderr)
    );
    assert!(resumed.status.success());
    assert_artifacts(&String::from_utf8_lossy(&resumed.stdout), &dll, &exe);
    {
        let log = log.lock();
        let resumed: Vec<_> = log.iter().filter(|e| e.phase == 2).collect();
        assert_eq!(resumed.len(), 1);
        let entry = resumed[0];
        assert_eq!(entry.digest, big.0);
        assert_eq!(entry.if_range.as_deref(), Some(validator.as_str()));
        assert_eq!(
            entry.range.as_deref(),
            Some(format!("bytes={saved}-").as_str())
        );
        assert_eq!(
            (entry.status, entry.declared, entry.emitted),
            (206, BIG_LEN - saved, BIG_LEN - saved)
        );
        println!(
            "wire (bytes yielded to HTTP body): {log:?}; peak active bodies={}",
            bodies.lock().1
        );
    }
    let state: serde_json::Value = serde_json::from_slice(&fs::read(&baseline).unwrap()).unwrap();
    assert_eq!(state["server_url"], origin);
    assert_eq!(state["mods"][0]["id"], id);
    let receipt = state["mods"][0]["files"].as_array().unwrap();
    assert_eq!(receipt.len(), 2);
    for (path, len, hash) in [(large_path, BIG_LEN, &big), (small_path, SMALL_LEN, &small)] {
        let entry = receipt.iter().find(|item| item["path"] == path).unwrap();
        assert_eq!(entry["length"], len);
        assert!(entry.get("sha256").is_none());
        assert_eq!(entry["hash"], hash.1);
        assert_eq!(entry["owned"], true);
    }
    println!("applied receipt: {state}");
    for (path, len, sha) in [
        (large_path, BIG_LEN, big.0.clone()),
        (small_path, SMALL_LEN, small.0.clone()),
    ] {
        let path = game_root.join(path);
        assert_eq!(fs::metadata(&path).unwrap().len(), len);
        assert_eq!(
            tokio::task::spawn_blocking(move || destination_hash(&path))
                .await
                .unwrap(),
            sha
        );
    }
    let fenced_game = tempfile::tempdir().unwrap();
    let fenced_state = fenced_game.path().join("BepInEx/config/Convoy/state.json");
    fs::create_dir_all(fenced_state.parent().unwrap()).unwrap();
    fs::write(&fenced_state, b"{}").unwrap();
    fs::write(fenced_game.path().join("local.txt"), b"fenced local file").unwrap();
    phase.store(3, Ordering::SeqCst);
    let fenced = run(fenced_game.path()).await.unwrap().unwrap();
    println!(
        "full transfer with final revision fence: {}\n{}{}",
        fenced.status,
        String::from_utf8_lossy(&fenced.stdout),
        String::from_utf8_lossy(&fenced.stderr)
    );
    assert_eq!(fenced.status.code(), Some(15));
    assert_artifacts(&String::from_utf8_lossy(&fenced.stdout), &dll, &exe);
    assert!(fence.load(Ordering::SeqCst));
    assert_eq!(fs::read(&fenced_state).unwrap(), b"{}");
    assert_eq!(
        fs::read(fenced_game.path().join("local.txt")).unwrap(),
        b"fenced local file"
    );
    assert!(
        !fenced_game.path().join(large_path).exists()
            && !fenced_game.path().join(small_path).exists()
    );
    {
        let log = log.lock();
        let full: Vec<_> = log.iter().filter(|w| w.phase == 3).collect();
        assert_eq!(full.len(), 2);
        assert!(full.iter().all(|w| w.range.is_none()
            && w.if_range.is_none()
            && w.status == 200
            && w.declared == w.emitted));
        assert_eq!(full.iter().map(|w| w.emitted).sum::<u64>(), LARGE_TOTAL);
        println!("full transfer body observations: {full:?}");
    }
    handle.stop(true).await;
    service.await.unwrap().unwrap();
    upstream_handle.stop(true).await;
    upstream_task.await.unwrap().unwrap();
    let common::TestApp {
        db,
        app_state,
        _tmp_dir,
        ..
    } = app;
    let dirs = (*app_state.dirs).clone();
    let config = app_state.config.clone();
    let prior_status = app_state.convoy_publisher.status();
    drop(app_state);
    let restart_start = Instant::now();
    let restarted =
        spt_quartermaster::convoy::publisher::Publisher::start(dirs, config, db).unwrap();
    let mut restarted_status = restarted.subscribe();
    let reopened = tokio::time::timeout(
        Duration::from_secs(360),
        restarted_status.wait_for(|s| {
            matches!(
                s,
                PublicationStatus::Ready { .. } | PublicationStatus::Failed { .. }
            )
        }),
    )
    .await
    .unwrap()
    .unwrap()
    .clone();
    assert_eq!(reopened, prior_status);
    assert!(matches!(
        reopened,
        PublicationStatus::Ready {
            bytes: LARGE_TOTAL,
            ..
        }
    ));
    println!(
        "publisher restart validation {:?}: {reopened:?}",
        restart_start.elapsed()
    );
}
