#![allow(clippy::unwrap_used)]
use crate::{container::ContainerManager, db::Database, dirs::QumaDirs};
use std::{
    io::{Read, Write},
    os::unix::net::UnixListener,
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc,
    },
};

pub(crate) struct Fixture {
    pub dirs: QumaDirs,
    pub manager: ContainerManager,
    pub db: Arc<parking_lot::Mutex<Database>>,
}

/// Populate real native ownership/quarantine for lifecycle-generation tests.
pub(crate) fn seed_generation(f: &Fixture, clients: &[u32]) {
    const REL: &str = "BepInEx/plugins/Excluded.dll";
    let db = f.db.lock();
    let id = db
        .insert_mod(None, None, "Excluded", None, "1", "local", None, None)
        .unwrap();
    db.save_groups_atomic(&[(
        "No Headless".into(),
        "no-headless".into(),
        "optional".into(),
        true,
        vec![id],
    )])
    .unwrap();
    let hash = format!("{:032x}", xxhash_rust::xxh3::xxh3_128(b"published"));
    db.insert_file(id, REL, Some(&hash), Some(9)).unwrap();
    let policy = super::PolicySnapshot::load(&db).unwrap();
    drop(db);
    for root in [f.dirs.headless_base.clone(), f.dirs.mod_overlay()] {
        std::fs::create_dir_all(root.join("BepInEx/plugins")).unwrap();
        std::fs::write(root.join(REL), b"published").unwrap();
    }
    for &client in clients {
        let upper = f.dirs.headless_upper(client);
        std::fs::create_dir_all(upper.join("BepInEx/plugins")).unwrap();
        std::fs::create_dir_all(f.dirs.headless_work(client)).unwrap();
        std::fs::write(upper.join(REL), b"published").unwrap();
        let guard = super::PolicyOperationGuard::acquire(&f.dirs, client).unwrap();
        super::reconcile_private_overlay(&f.dirs, &policy, client, &guard).unwrap();
        let journal: serde_json::Value = serde_json::from_slice(
            &std::fs::read(
                f.dirs
                    .root
                    .join(format!("headless-policy/client-{client}/journal.json")),
            )
            .unwrap(),
        )
        .unwrap();
        assert!(journal["entries"][REL]["mask"].is_object());
    }
}

/// Real production orchestration in an isolated child; only the external Docker
/// HTTP boundary is simulated. Every start verifies reconciliation and lock hold.
pub(crate) fn run<F, Fut>(name: &str, case: &str, body: F)
where
    F: FnOnce(Fixture) -> Fut,
    Fut: std::future::Future<Output = ()>,
{
    if let Some(root) = std::env::var_os("QUMA_TASK2_ROOT") {
        if std::env::var("QUMA_TASK2_CASE").as_deref() != Ok(case) {
            return;
        }
        let dirs = QumaDirs::from_root(root.into());
        let db = Arc::new(parking_lot::Mutex::new(
            Database::open(&dirs.db_path()).unwrap(),
        ));
        let rt = tokio::runtime::Runtime::new().unwrap();
        rt.block_on(body(Fixture {
            dirs,
            manager: ContainerManager::new(1).unwrap(),
            db,
        }));
        return;
    }
    let tmp = tempfile::tempdir().unwrap();
    let dirs = QumaDirs::from_root(tmp.path().to_owned());
    for p in [
        dirs.headless_base.clone(),
        dirs.headless_upper(1),
        dirs.headless_work(1),
        dirs.mod_overlay(),
    ] {
        std::fs::create_dir_all(p).unwrap();
    }
    let db = Database::open(&dirs.db_path()).unwrap();
    drop(db);
    if case == "preconfigured" {
        let cfg = dirs
            .headless_upper(1)
            .join("BepInEx/config/com.fika.core.cfg");
        std::fs::create_dir_all(cfg.parent().unwrap()).unwrap();
        std::fs::write(cfg, "[Network]\nUDPPort = 0\n").unwrap();
    }
    if case == "cancel" {
        use std::os::unix::fs::PermissionsExt;
        let bin = tmp.path().join("bin");
        std::fs::create_dir(&bin).unwrap();
        let script = bin.join("fuse-overlayfs");
        std::fs::write(&script,"#!/bin/sh\n: > \"$QUMA_TASK2_ROOT/entered\"\nwhile [ ! -f \"$QUMA_TASK2_ROOT/release\" ]; do sleep 0.01; done\nexec /usr/bin/fuse-overlayfs \"$@\"\n").unwrap();
        std::fs::set_permissions(script, std::fs::Permissions::from_mode(0o700)).unwrap();
    }
    let socket = tmp.path().join("docker.sock");
    let listener = UnixListener::bind(&socket).unwrap();
    listener.set_nonblocking(true).unwrap();
    let done = Arc::new(AtomicBool::new(false));
    let stop = done.clone();
    let calls = Arc::new(std::sync::Mutex::new(Vec::new()));
    let recorded = calls.clone();
    let scenario = case.to_owned();
    let root = tmp.path().to_owned();
    let join = std::thread::spawn(move || {
        let dirs = QumaDirs::from_root(root);
        let mut created = false;
        let mut running = scenario.starts_with("memory");
        let mut starts = 0;
        while !stop.load(Ordering::SeqCst) {
            let (mut stream, _) = match listener.accept() {
                Ok(s) => s,
                Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                    std::thread::sleep(std::time::Duration::from_millis(2));
                    continue;
                }
                Err(e) => panic!("{e}"),
            };
            stream
                .set_read_timeout(Some(std::time::Duration::from_secs(10)))
                .unwrap();
            let mut request = Vec::new();
            let mut buf = [0; 4096];
            let header_end = loop {
                let n = stream.read(&mut buf).unwrap();
                if n == 0 {
                    break request.len();
                }
                request.extend_from_slice(&buf[..n]);
                if let Some(pos) = request.windows(4).position(|b| b == b"\r\n\r\n") {
                    break pos + 4;
                }
            };
            let header = String::from_utf8_lossy(&request[..header_end]).to_string();
            let length = header
                .lines()
                .find_map(|s| {
                    s.to_ascii_lowercase()
                        .strip_prefix("content-length:")
                        .and_then(|v| v.trim().parse::<usize>().ok())
                })
                .unwrap_or(0);
            while request.len() < header_end + length {
                let n = stream.read(&mut buf).unwrap();
                if n == 0 {
                    break;
                }
                request.extend_from_slice(&buf[..n]);
            }
            let line = header.lines().next().unwrap_or("").to_owned();
            recorded.lock().unwrap().push(line.clone());
            let mut code = 200;
            let payload = if scenario.starts_with("generation-") && line.contains("/version") {
                // Bound whole convergence before external image/Fika acquisition.
                "{\"Version\":\"3.0\"}".into()
            } else if scenario.starts_with("generation-") && line.contains("/containers/json") {
                "[{\"Id\":\"owned-id\",\"Names\":[\"/fika-headless-1\"]}]".into()
            } else if scenario.starts_with("generation-") && line.starts_with("DELETE ") {
                created = false;
                running = false;
                code = 204;
                String::new()
            } else if line.contains("/create?") {
                created = true;
                let request: serde_json::Value =
                    serde_json::from_slice(&request[header_end..]).unwrap();
                assert!(request["HostConfig"]["Binds"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .any(|b| b.as_str().unwrap().contains(":/opt/tarkov:O,")));
                "{\"Id\":\"owned-id\",\"Warnings\":[]}".into()
            } else if line.contains("/json") {
                let status = if running {
                    "running"
                } else if created && starts == 0 {
                    "created"
                } else {
                    "exited"
                };
                let binds = if created && starts == 0 {
                    vec![]
                } else {
                    vec![format!(
                        "{}:/opt/tarkov:O,upperdir={},workdir={}",
                        dirs.headless_base.display(),
                        dirs.headless_upper(1).display(),
                        dirs.headless_work(1).display()
                    )]
                };
                serde_json::json!({"Id":if scenario=="wrong-id"&&created{"foreign-id"}else{"owned-id"},"Name":"/fika-headless-1","Config":{"Labels":{"managed-by":"quma","quma.managed-by":"quartermaster-clients","quma.client.index":"1"}},"State":{"Status":status,"Running":running,"Paused":false,"Restarting":false,"Pid":if running{123}else{0}},"HostConfig":{"Binds":binds},"Mounts":[]}).to_string()
            } else if line.contains("/start") {
                assert!(
                    dirs.root.join("headless-policy/client-1").is_dir(),
                    "start before preparation"
                );
                if scenario == "native" || scenario.starts_with("generation-") {
                    let journal = std::fs::read_to_string(
                        dirs.root.join("headless-policy/client-1/journal.json"),
                    )
                    .unwrap();
                    assert!(
                        journal.contains("hidden"),
                        "native exclusion unfinished at start"
                    );
                }
                assert!(
                    super::PolicyOperationGuard::acquire(&dirs, 1).is_err(),
                    "start without operation guard"
                );
                starts += 1;
                if scenario == "boot-policy-change" && starts == 1 {
                    Database::open(&dirs.db_path())
                        .unwrap()
                        .conn()
                        .execute("DROP TABLE mod_groups", [])
                        .unwrap();
                }
                running = true;
                code = 204;
                let cfg = dirs
                    .headless_upper(1)
                    .join("BepInEx/config/com.fika.core.cfg");
                std::fs::create_dir_all(cfg.parent().unwrap()).unwrap();
                std::fs::write(cfg, "[Network]\nUDPPort = 0\n").unwrap();
                String::new()
            } else if line.contains("/stop") {
                running = false;
                code = 204;
                String::new()
            } else if line.contains("/wait") {
                if scenario == "crash" {
                    "{\"StatusCode\":1,\"Error\":{\"Message\":\"crash\"}}".into()
                } else {
                    "{\"StatusCode\":0}".into()
                }
            } else {
                code = 500;
                "{\"message\":\"unexpected API\"}".into()
            };
            let _=write!(stream,"HTTP/1.1 {code} Result\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{payload}",payload.len());
        }
    });
    let out = std::process::Command::new(std::env::current_exe().unwrap())
        .args(["--exact", name, "--nocapture"])
        .env("QUMA_TASK2_ROOT", tmp.path())
        .env("QUMA_TASK2_CASE", case)
        .env(
            "PATH",
            format!(
                "{}:{}",
                tmp.path().join("bin").display(),
                std::env::var("PATH").unwrap()
            ),
        )
        .env("DOCKER_HOST", format!("unix://{}", socket.display()))
        .output()
        .unwrap();
    done.store(true, Ordering::SeqCst);
    join.join().unwrap();
    assert!(
        out.status.success(),
        "{case}: {}\n{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
    let calls = calls.lock().unwrap();
    if case.starts_with("generation-") {
        assert!(
            calls.iter().any(|s| s.starts_with("DELETE ")),
            "must exercise container teardown"
        );
        assert_eq!(
            calls.iter().filter(|s| s.contains("/start")).count(),
            1,
            "replacement starts exactly once"
        );
    }
    if case == "boot-policy-change" {
        assert_eq!(calls.iter().filter(|s| s.contains("/start")).count(), 1);
        assert!(calls.iter().any(|s| s.contains("/stop")));
    }
    if [
        "valid",
        "native",
        "preconfigured",
        "memory",
        "graceful-valid",
    ]
    .contains(&case)
    {
        let expected = if case == "valid" || case == "native" {
            2
        } else {
            1
        };
        assert_eq!(
            calls.iter().filter(|s| s.contains("/start")).count(),
            expected,
            "successful orchestration omitted a start: {calls:?}"
        );
    }
    if case.starts_with("busy-") {
        assert!(
            calls.is_empty(),
            "busy operation made external calls: {calls:?}"
        );
    }
    if [
        "invalid",
        "clean",
        "crash",
        "wrong-id",
        "cancel",
        "memory-invalid",
        "memory-raid",
        "memory-manual",
        "graceful-invalid",
        "graceful-raid",
        "graceful-manual",
    ]
    .contains(&case)
    {
        assert!(
            !calls.iter().any(|s| s.contains("/start")),
            "failed preparation started: {calls:?}"
        );
    }
}
