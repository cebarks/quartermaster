//! Real private Podman storage + service, real native FUSE; no existing client.
#![allow(clippy::unwrap_used)]
use crate::{
    container::{ContainerManager, CreateContainerOpts, OverlayMount},
    db::Database,
    dirs::QumaDirs,
};
use std::{
    path::PathBuf,
    process::{Child, Command},
    sync::Arc,
};
struct Service(Child);
impl Drop for Service {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

#[test]
fn native_never_started_creation_prepares_without_boot_or_relaxed_existing_inspection() {
    if let Some(root) = std::env::var_os("QUMA_NATIVE_TASK2_ROOT") {
        let dirs = QumaDirs::from_root(PathBuf::from(root).join("installation"));
        for p in [
            dirs.headless_base.clone(),
            dirs.headless_upper(1),
            dirs.headless_work(1),
            dirs.mod_overlay(),
        ] {
            std::fs::create_dir_all(p).unwrap();
        }
        let db = Database::open_in_memory().unwrap();
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
        let hash = format!("{:016x}", xxhash_rust::xxh3::xxh3_64(b"published"));
        db.insert_file(id, "BepInEx/plugins/Excluded.dll", Some(&hash), Some(9))
            .unwrap();
        for p in [
            dirs.headless_base.clone(),
            dirs.headless_upper(1),
            dirs.mod_overlay(),
        ] {
            std::fs::create_dir_all(p.join("BepInEx/plugins")).unwrap();
            std::fs::write(p.join("BepInEx/plugins/Excluded.dll"), b"published").unwrap();
        }
        let rt = tokio::runtime::Runtime::new().unwrap();
        rt.block_on(async {
            let manager = ContainerManager::new(1).unwrap();
            let guard = Arc::new(super::PolicyOperationGuard::acquire(&dirs, 1).unwrap());
            let opts = CreateContainerOpts {
                name: "fika-headless-1".into(),
                image: "localhost/quma-task2-fixture:latest".into(),
                env: vec![],
                volumes: vec![],
                overlay_mounts: vec![OverlayMount {
                    lower_dir: dirs.headless_base.clone(),
                    upper_dir: dirs.headless_upper(1),
                    work_dir: dirs.headless_work(1),
                }],
                ports: vec![],
                labels: vec![
                    ("quma.managed-by".into(), "quartermaster-clients".into()),
                    ("quma.client.index".into(), "1".into()),
                ],
                user: None,
                healthcheck: None,
                devices: vec![],
                security_opt: vec![],
                cpuset_cpus: None,
                cpuset_mems: None,
                network_mode: Some("none".into()),
            };
            let created =
                super::runtime::create_owned_client(&manager, &dirs, 1, opts, guard.clone())
                    .await
                    .unwrap();
            let info = manager.inspect("fika-headless-1").await.unwrap();
            assert!(
                info.host_config
                    .as_ref()
                    .unwrap()
                    .binds
                    .as_ref()
                    .is_none_or(Vec::is_empty),
                "native fixture must exercise empty preboot bindings"
            );
            assert!(
                super::runtime::inspect_stopped_client(&manager, &dirs, 1)
                    .await
                    .is_err(),
                "existing-container inspection must remain strict"
            );
            super::runtime::prepare_created_client(
                &manager,
                &dirs,
                super::PolicySnapshot::load(&db).unwrap(),
                1,
                created,
            )
            .await
            .unwrap();
            let info = manager.inspect("fika-headless-1").await.unwrap();
            assert_eq!(info.state.unwrap().status.unwrap().as_ref(), "created");
            let journal =
                std::fs::read_to_string(dirs.root.join("headless-policy/client-1/journal.json"))
                    .unwrap();
            assert!(journal.contains("hidden"));
            assert_eq!(
                std::fs::read(dirs.headless_base.join("BepInEx/plugins/Excluded.dll")).unwrap(),
                b"published"
            );
            assert_eq!(
                std::fs::read(dirs.mod_overlay().join("BepInEx/plugins/Excluded.dll")).unwrap(),
                b"published"
            );
        });
        return;
    }
    let tmp = tempfile::tempdir().unwrap();
    let mut base = vec![
        "--root".to_owned(),
        tmp.path().join("storage").display().to_string(),
        "--runroot".into(),
        tmp.path().join("run").display().to_string(),
        "--storage-driver".into(),
        "vfs".into(),
        "--events-backend".into(),
        "file".into(),
    ];
    let tar = tmp.path().join("empty.tar");
    assert!(Command::new("tar")
        .args(["-cf", tar.to_str().unwrap(), "--files-from", "/dev/null"])
        .status()
        .unwrap()
        .success());
    let import = Command::new("podman")
        .args(&base)
        .args([
            "import",
            "--change",
            "CMD [\"/not-executed\"]",
            tar.to_str().unwrap(),
            "localhost/quma-task2-fixture:latest",
        ])
        .output()
        .unwrap();
    assert!(
        import.status.success(),
        "native fixture import: {}",
        String::from_utf8_lossy(&import.stderr)
    );
    let socket = tmp.path().join("podman.sock");
    base.extend([
        "system".into(),
        "service".into(),
        "--time=0".into(),
        format!("unix://{}", socket.display()),
    ]);
    let log = std::fs::File::create(tmp.path().join("podman.log")).unwrap();
    let _service = Service(
        Command::new("podman")
            .args(&base)
            .stdout(log.try_clone().unwrap())
            .stderr(log)
            .spawn()
            .unwrap(),
    );
    for _ in 0..500 {
        if socket.exists() {
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(10));
    }
    assert!(
        socket.exists(),
        "native fixture service failed: {}",
        std::fs::read_to_string(tmp.path().join("podman.log")).unwrap()
    );
    let out=Command::new(std::env::current_exe().unwrap()).args(["--exact","headless::exclusions::native_tests::native_never_started_creation_prepares_without_boot_or_relaxed_existing_inspection","--nocapture"]).env("QUMA_NATIVE_TASK2_ROOT",tmp.path()).env("DOCKER_HOST",format!("unix://{}",socket.display())).output().unwrap();
    assert!(
        out.status.success(),
        "native creation: {}\n{}\nservice: {}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr),
        std::fs::read_to_string(tmp.path().join("podman.log")).unwrap()
    );
}
