//! Behavioral lifecycle tests against a disposable Docker API, never a daemon.
#![allow(clippy::unwrap_used)]
use spt_quartermaster::{
    client::{ClientHealth, ClientState, ContainerStatus},
    config::Config,
    container::ContainerManager,
    db::Database,
    dirs::QumaDirs,
    forge::client::ForgeClient,
    headless::{service::LifecycleAction, HeadlessService},
};
use std::{
    io::{Read, Write},
    os::unix::net::UnixListener,
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc,
    },
};

#[test]
fn explicit_lifecycle_enforces_current_policy_before_start() {
    if let Some(root) = std::env::var_os("QUMA_LIFECYCLE_ROOT") {
        let dirs = Arc::new(QumaDirs::from_root(root.into()));
        let db = Arc::new(parking_lot::Mutex::new(
            Database::open(&dirs.db_path()).unwrap(),
        ));
        if std::env::var("QUMA_LIFECYCLE_CASE")
            .unwrap()
            .contains("invalid")
        {
            db.lock()
                .conn()
                .execute("DROP TABLE mod_groups", [])
                .unwrap();
        }
        let states = Arc::new(tokio::sync::RwLock::new(vec![ClientState {
            index: 1,
            container_name: "fika-headless-1".into(),
            container_status: ContainerStatus::Stopped,
            fika_status: None,
            players: vec![],
            cpu_percent: None,
            memory_mb: None,
            restart_count: 0,
            last_restart: None,
            health: ClientHealth::Down,
            restarting: false,
            consecutive_failures: 0,
            first_seen: chrono::Utc::now(),
            manually_stopped: true,
            profile_id: Some("profile".into()),
            prev_fika_status: None,
        }]));
        let (events, _) = tokio::sync::broadcast::channel(8);
        let svc = HeadlessService::new(
            ContainerManager::new(1).unwrap(),
            Arc::new(parking_lot::RwLock::new(Config::default())),
            dirs.config_path(),
            Arc::new(parking_lot::Mutex::new(())),
            dirs,
            db.clone(),
            Arc::new(AtomicBool::new(false)),
            states.clone(),
            None,
            Arc::new(parking_lot::Mutex::new(())),
            ForgeClient::new().unwrap(),
            events,
        );
        let rt = tokio::runtime::Runtime::new().unwrap();
        rt.block_on(async {
            let case = std::env::var("QUMA_LIFECYCLE_CASE").unwrap();
            let action = if case.starts_with("restart-") {
                LifecycleAction::Restart
            } else {
                LifecycleAction::Start
            };
            let result = svc.client_lifecycle(1, action).await;
            if std::env::var("QUMA_LIFECYCLE_CASE")
                .unwrap()
                .contains("invalid")
            {
                assert!(result.is_err(), "invalid policy must prevent start");
                assert!(states.read().await[0].manually_stopped);
            } else {
                result.unwrap();
                assert!(!states.read().await[0].manually_stopped);
            }
            if case == "current" {
                svc.client_lifecycle(1, LifecycleAction::Stop)
                    .await
                    .unwrap();
                db.lock()
                    .conn()
                    .execute("DROP TABLE mod_groups", [])
                    .unwrap();
                assert!(svc
                    .client_lifecycle(1, LifecycleAction::Start)
                    .await
                    .is_err());
                assert!(states.read().await[0].manually_stopped);
            }
        });
        return;
    }
    for case in [
        "invalid",
        "valid",
        "restart-invalid",
        "restart-valid",
        "current",
    ] {
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
        drop(Database::open(&dirs.db_path()).unwrap());
        let socket = tmp.path().join("docker.sock");
        let listener = UnixListener::bind(&socket).unwrap();
        listener.set_nonblocking(true).unwrap();
        let done = Arc::new(AtomicBool::new(false));
        let stop = done.clone();
        let root = tmp.path().to_owned();
        let calls = Arc::new(std::sync::Mutex::new(Vec::new()));
        let recorded = calls.clone();
        let join = std::thread::spawn(move || {
            let mut running = case.starts_with("restart-");
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
                    .set_read_timeout(Some(std::time::Duration::from_secs(5)))
                    .unwrap();
                let mut bytes = Vec::new();
                let mut buf = [0; 4096];
                while !bytes.windows(4).any(|b| b == b"\r\n\r\n") {
                    let n = stream.read(&mut buf).unwrap();
                    if n == 0 {
                        break;
                    }
                    bytes.extend_from_slice(&buf[..n]);
                }
                let request = String::from_utf8(bytes).unwrap();
                let line = request.lines().next().unwrap().to_owned();
                recorded.lock().unwrap().push(line.clone());
                let (code, body) = if line.contains("/json") {
                    (200,serde_json::json!({"Id":"owned-id","Name":"/fika-headless-1","Config":{"Labels":{"managed-by":"quma","quma.managed-by":"quartermaster-clients","quma.client.index":"1"}},"State":{"Status":if running{"running"}else{"exited"},"Running":running,"Paused":false,"Restarting":false,"Pid":if running{123}else{0}},"HostConfig":{"Binds":[format!("{}:/opt/tarkov:O,upperdir={},workdir={}",root.join("headless").display(),root.join("overlays/headless/client-1/upper").display(),root.join("overlays/headless/client-1/work").display())]}}).to_string())
                } else if line.contains("/start") {
                    assert!(
                        root.join("headless-policy/client-1").is_dir(),
                        "start before reconciliation"
                    );
                    assert!(
                        spt_quartermaster::headless::exclusions::PolicyOperationGuard::acquire(
                            &QumaDirs::from_root(root.clone()),
                            1
                        )
                        .is_err(),
                        "start without guard"
                    );
                    running = true;
                    (204, String::new())
                } else if line.contains("/stop") {
                    assert!(
                        spt_quartermaster::headless::exclusions::PolicyOperationGuard::acquire(
                            &QumaDirs::from_root(root.clone()),
                            1
                        )
                        .is_err(),
                        "stop outside operation guard"
                    );
                    running = false;
                    (204, String::new())
                } else {
                    (500, "{\"message\":\"unexpected lifecycle request\"}".into())
                };
                write!(stream,"HTTP/1.1 {code} Result\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",body.len()).unwrap();
            }
        });
        let output = std::process::Command::new(std::env::current_exe().unwrap())
            .args([
                "--exact",
                "explicit_lifecycle_enforces_current_policy_before_start",
                "--nocapture",
            ])
            .env("QUMA_LIFECYCLE_ROOT", tmp.path())
            .env("QUMA_LIFECYCLE_CASE", case)
            .env("DOCKER_HOST", format!("unix://{}", socket.display()))
            .output()
            .unwrap();
        done.store(true, Ordering::SeqCst);
        join.join().unwrap();
        assert!(
            output.status.success(),
            "case {case}: {}",
            String::from_utf8_lossy(&output.stdout)
        );
        let calls = calls.lock().unwrap();
        if case.contains("invalid") {
            assert!(!calls.iter().any(|s| s.contains("/start")));
        } else {
            assert!(calls.iter().any(|s| s.contains("/start")));
            assert_eq!(
                calls.iter().filter(|s| s.contains("/start")).count(),
                1,
                "must not start after later policy failure"
            );
            assert!(
                tmp.path().join("headless-policy").exists(),
                "safe start must reconcile"
            );
        }
    }
}
