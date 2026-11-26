//! Real CLI + fake inspect-only Docker endpoint + real filesystem/FUSE engine.
#![allow(clippy::unwrap_used)]
use spt_quartermaster::{db::Database, dirs::QumaDirs};
use std::collections::BTreeMap;
use std::io::{Read, Write};
use std::os::unix::net::UnixListener;
use std::path::Path;
use std::sync::{
    atomic::{AtomicBool, Ordering},
    Arc, Mutex,
};

struct Server {
    _tmp: tempfile::TempDir,
    socket: std::path::PathBuf,
    calls: Arc<Mutex<Vec<String>>>,
    done: Arc<AtomicBool>,
    join: Option<std::thread::JoinHandle<()>>,
}
impl Server {
    fn new(status: u16, body: serde_json::Value) -> Self {
        let tmp = tempfile::tempdir().unwrap();
        let socket = tmp.path().join("docker.sock");
        let listener = UnixListener::bind(&socket).unwrap();
        listener.set_nonblocking(true).unwrap();
        let calls = Arc::new(Mutex::new(Vec::new()));
        let done = Arc::new(AtomicBool::new(false));
        let (c, d) = (calls.clone(), done.clone());
        let join = std::thread::spawn(move || {
            while !d.load(Ordering::SeqCst) {
                let (mut stream, _) = match listener.accept() {
                    Ok(s) => s,
                    Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                        std::thread::sleep(std::time::Duration::from_millis(5));
                        continue;
                    }
                    Err(e) => panic!("{e}"),
                };
                stream
                    .set_read_timeout(Some(std::time::Duration::from_secs(3)))
                    .unwrap();
                let mut request = Vec::new();
                let mut b = [0; 1024];
                while !request.windows(4).any(|x| x == b"\r\n\r\n") {
                    let n = stream.read(&mut b).unwrap();
                    if n == 0 {
                        break;
                    }
                    request.extend_from_slice(&b[..n]);
                }
                let text = String::from_utf8(request).unwrap();
                let line = text.lines().next().unwrap_or("").to_owned();
                let mut recorded = c.lock().unwrap();
                let call_index = recorded.len();
                recorded.push(line.clone());
                drop(recorded);
                let allowed =
                    line.starts_with("GET ") && line.contains("/containers/fika-headless-1/json");
                let code = if allowed { status } else { 500 };
                let payload = if allowed {
                    if let Some(responses) = body.as_array() {
                        responses[call_index.min(responses.len() - 1)].to_string()
                    } else {
                        body.to_string()
                    }
                } else {
                    "{\"message\":\"unexpected control/API call\"}".into()
                };
                write!(stream,"HTTP/1.1 {code} Result\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{payload}",payload.len()).unwrap();
            }
        });
        Self {
            _tmp: tmp,
            socket,
            calls,
            done,
            join: Some(join),
        }
    }
}
impl Drop for Server {
    fn drop(&mut self) {
        self.done.store(true, Ordering::SeqCst);
        self.join.take().unwrap().join().unwrap();
    }
}

fn snapshot(root: &Path) -> BTreeMap<std::path::PathBuf, Vec<u8>> {
    fn walk(root: &Path, p: &Path, out: &mut BTreeMap<std::path::PathBuf, Vec<u8>>) {
        for e in std::fs::read_dir(p).unwrap() {
            let e = e.unwrap();
            let p = e.path();
            if p.is_dir() {
                out.insert(p.strip_prefix(root).unwrap().to_owned(), vec![]);
                walk(root, &p, out);
            } else {
                out.insert(
                    p.strip_prefix(root).unwrap().to_owned(),
                    std::fs::read(p).unwrap(),
                );
            }
        }
    }
    let mut out = BTreeMap::new();
    walk(root, root, &mut out);
    out
}
struct Fixture {
    _tmp: tempfile::TempDir,
    dirs: QumaDirs,
}
impl Fixture {
    fn new() -> Self {
        let tmp = tempfile::tempdir().unwrap();
        let dirs = QumaDirs::from_root(tmp.path().to_owned());
        for p in [
            dirs.spt_server.clone(),
            dirs.headless_base.clone(),
            dirs.headless_upper(1),
            dirs.headless_work(1),
            dirs.mod_overlay(),
        ] {
            std::fs::create_dir_all(p).unwrap();
        }
        let db = Database::open(&dirs.db_path()).unwrap();
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
        let bytes = b"published";
        let hash = format!("{:032x}", xxhash_rust::xxh3::xxh3_128(bytes));
        db.insert_file(
            id,
            "BepInEx/plugins/Excluded.dll",
            Some(&hash),
            Some(bytes.len() as i64),
        )
        .unwrap();
        db.conn()
            .pragma_update(None, "journal_mode", "DELETE")
            .unwrap();
        drop(db);
        for root in [dirs.headless_upper(1), dirs.mod_overlay()] {
            std::fs::create_dir_all(root.join("BepInEx/plugins")).unwrap();
            std::fs::write(root.join("BepInEx/plugins/Excluded.dll"), bytes).unwrap();
        }
        std::fs::write(
            dirs.config_path(),
            b"intentionally invalid TOML; command must not read config",
        )
        .unwrap();
        Self { _tmp: tmp, dirs }
    }
    fn inspect(&self) -> serde_json::Value {
        serde_json::json!({"Id":"original-id","Name":"/fika-headless-1","Config":{"Labels":{"managed-by":"quma","quma.managed-by":"quartermaster-clients","quma.client.index":"1"},"Env":["PROFILE_ID=must-not-print"]},"State":{"Status":"exited","Running":false,"Paused":false,"Restarting":false,"Pid":0},"HostConfig":{"Binds":[format!("{}:/opt/tarkov:lowerdir={},upperdir={},workdir={},private,userxattr",self.dirs.root.join("storage/overlay-containers/original-id/userdata/overlay/123/merge").display(),self.dirs.headless_base.display(),self.dirs.headless_upper(1).display(),self.dirs.headless_work(1).display())]}})
    }
    fn run(&self, server: &Server, check: bool) -> std::process::Output {
        let mut cmd = std::process::Command::new(env!("CARGO_BIN_EXE_quma"));
        cmd.env_remove("QUMA_DIR").env_remove("QUMA_SPT_DIR");
        cmd.current_dir(self.dirs.root.parent().unwrap());
        cmd.args([
            "--quma-dir",
            self.dirs.root.to_str().unwrap(),
            "headless",
            "reconcile-exclusions",
            "1",
        ]);
        if check {
            cmd.arg("--check");
        }
        cmd.env("DOCKER_HOST", format!("unix://{}", server.socket.display()))
            .output()
            .unwrap()
    }
}

#[test]
fn offline_cli_check_is_read_only_and_bypasses_fika_and_tokens() {
    let f = Fixture::new();
    let server = Server::new(200, f.inspect());
    let before = snapshot(&f.dirs.root);
    let output = f.run(&server, true);
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let report: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(report["check_only"], true);
    assert_eq!(report["changed"], 1);
    assert_eq!(before, snapshot(&f.dirs.root));
    assert!(!String::from_utf8_lossy(&output.stdout).contains("must-not-print"));
    assert_eq!(server.calls.lock().unwrap().len(), 2);
}

#[test]
fn offline_cli_requires_stop_and_inspection_errors_fail_without_writes() {
    for code in [200, 404, 500] {
        let f = Fixture::new();
        let mut info = f.inspect();
        info["State"]["Running"] = true.into();
        info["State"]["Status"] = "running".into();
        let server = Server::new(code, info);
        let before = snapshot(&f.dirs.root);
        let output = f.run(&server, true);
        assert!(!output.status.success());
        assert_eq!(before, snapshot(&f.dirs.root));
        assert_eq!(server.calls.lock().unwrap().len(), 1);
    }
}

#[test]
fn offline_cli_applies_real_upper_exclusion_without_any_control_calls() {
    let f = Fixture::new();
    let server = Server::new(200, f.inspect());
    let original_db = std::fs::read(f.dirs.db_path()).unwrap();
    let output = f.run(&server, false);
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let report: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(report["check_only"], false);
    assert_eq!(report["changed"], 1);
    assert!(!f
        .dirs
        .headless_upper(1)
        .join("BepInEx/plugins/Excluded.dll")
        .exists());
    assert_eq!(
        std::fs::read(f.dirs.mod_overlay().join("BepInEx/plugins/Excluded.dll")).unwrap(),
        b"published"
    );
    assert_eq!(std::fs::read(f.dirs.db_path()).unwrap(), original_db);
    assert!(f
        .dirs
        .root
        .join("headless-policy/client-1/journal.json")
        .is_file());
    assert_eq!(server.calls.lock().unwrap().len(), 2);
}

#[test]
fn offline_cli_rechecks_original_container_identity() {
    let f = Fixture::new();
    let mut replacement = f.inspect();
    replacement["Id"] = "replacement-container".into();
    let server = Server::new(200, serde_json::json!([f.inspect(), replacement]));
    let before = snapshot(&f.dirs.root);
    let output = f.run(&server, true);
    assert!(!output.status.success());
    assert_eq!(before, snapshot(&f.dirs.root));
    assert_eq!(server.calls.lock().unwrap().len(), 2);
}

#[test]
fn offline_cli_engine_error_returns_nonzero_without_start() {
    let f = Fixture::new();
    std::fs::write(
        f.dirs
            .headless_upper(1)
            .join("BepInEx/plugins/Excluded.dll"),
        b"custom operator data",
    )
    .unwrap();
    let server = Server::new(200, f.inspect());
    let output = f.run(&server, false);
    assert!(!output.status.success());
    assert_eq!(
        std::fs::read(
            f.dirs
                .headless_upper(1)
                .join("BepInEx/plugins/Excluded.dll")
        )
        .unwrap(),
        b"custom operator data"
    );
    assert_eq!(server.calls.lock().unwrap().len(), 1);
}

#[test]
fn offline_cli_rejects_zero_and_exposes_help() {
    let output = std::process::Command::new(env!("CARGO_BIN_EXE_quma"))
        .args(["headless", "reconcile-exclusions", "0", "--check"])
        .output()
        .unwrap();
    assert!(!output.status.success());
    let help = std::process::Command::new(env!("CARGO_BIN_EXE_quma"))
        .args(["headless", "reconcile-exclusions", "--help"])
        .output()
        .unwrap();
    assert!(help.status.success());
    assert!(String::from_utf8_lossy(&help.stdout).contains("--check"));
}
