use actix_web::{web, App, HttpServer};
use parking_lot::{Mutex, RwLock};
#[path = "common/convoy_v2.rs"]
mod publication;
use serde_json::Value;
use spt_quartermaster::{
    config::{Config, ConvoyConfig},
    convoy::{
        object_store::ObjectStore,
        publisher::{build_snapshot, CatalogInput, PublicationStatus, PublishMode, Publisher},
    },
    db::Database,
    dirs::QumaDirs,
    web::handlers::convoy_v2::configure_v2,
};
use std::{fs, net::TcpListener, path::Path, sync::Arc};

const ABC: &str = "06b05ab6733a618578af5f94892f3950";
const EMPTY: &str = "99aa06d3014798d86001c324468d497f";
const PLUGIN: &str = "BepInEx/plugins/Test/item.dll";
const MANAGED: &str = "EscapeFromTarkov_Data/Managed/Dependency.dll";

struct Fixture {
    _tmp: tempfile::TempDir,
    dirs: QumaDirs,
    db: Arc<Mutex<Database>>,
    id: i64,
    config: Arc<RwLock<Config>>,
}

fn write(root: &Path, path: &str, content: &[u8]) {
    let p = root.join(path);
    fs::create_dir_all(p.parent().unwrap()).unwrap();
    fs::write(p, content).unwrap();
}

impl Fixture {
    fn new(nested: bool) -> Self {
        let tmp = tempfile::tempdir().unwrap();
        let dirs = if nested {
            QumaDirs::from_root(tmp.path().to_owned())
        } else {
            QumaDirs::from_legacy(tmp.path().to_owned())
        };
        fs::create_dir_all(&dirs.spt_server).unwrap();
        let db = Database::open(&dirs.db_path()).unwrap();
        let id = db
            .insert_mod(None, None, "Test", None, "1", "local", None, None)
            .unwrap();
        let config = Arc::new(RwLock::new(Config {
            convoy: Some(ConvoyConfig {
                enabled: true,
                ..ConvoyConfig::default()
            }),
            ..Config::default()
        }));
        Self {
            _tmp: tmp,
            dirs,
            db: Arc::new(Mutex::new(db)),
            id,
            config,
        }
    }

    fn track(&self, id: i64, path: &str, bytes: &[u8]) {
        let hash = format!("{:032x}", xxhash_rust::xxh3::xxh3_128(bytes));
        self.db
            .lock()
            .insert_file(id, path, Some(&hash), Some(bytes.len() as i64))
            .unwrap();
    }

    fn add(&self, path: &str, bytes: &[u8]) {
        write(&self.dirs.spt_server, path, bytes);
        self.track(self.id, path, bytes);
    }

    fn publisher(&self, cap: u64) -> Publisher {
        self.config.write().convoy.as_mut().unwrap().max_store_bytes = cap;
        Publisher::start(self.dirs.clone(), self.config.clone(), self.db.clone()).unwrap()
    }

    fn input(&self) -> CatalogInput {
        CatalogInput::capture(
            &self.db.lock(),
            &self.dirs,
            self.config.read().convoy.as_ref().unwrap(),
        )
        .unwrap()
    }

    fn store(&self, cap: u64) -> ObjectStore {
        ObjectStore::new(&self.dirs.convoy_store_dir(), cap).unwrap()
    }
}

async fn serve(publisher: Arc<Publisher>) -> (String, actix_web::dev::ServerHandle) {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let url = format!("http://{}/quma/convoy/v3", listener.local_addr().unwrap());
    let publisher = web::Data::from(publisher);
    let server = HttpServer::new(move || {
        App::new()
            .app_data(publisher.clone())
            .service(web::scope("/quma").configure(configure_v2))
    })
    .workers(1)
    .listen(listener)
    .unwrap()
    .run();
    let handle = server.handle();
    actix_web::rt::spawn(server);
    (url, handle)
}

#[actix_web::test]
async fn v3_publishes_fresh_xxh128_object_and_rejects_v2() {
    let f = Fixture::new(true);
    write(&f.dirs.spt_server, PLUGIN, b"abc");
    let digest = "06b05ab6733a618578af5f94892f3950";
    f.db.lock()
        .insert_file(f.id, PLUGIN, Some(digest), Some(3))
        .unwrap();
    let publisher = Arc::new(f.publisher(64));
    let generation = publication::ready(&publisher).await;
    let (base, server) = serve(publisher).await;
    let client = reqwest::Client::builder().no_proxy().build().unwrap();
    let v3 = base.replace("/v2", "/v3");
    let catalog = client.get(format!("{v3}/catalog")).send().await.unwrap();
    let status = catalog.status();
    let body = catalog.bytes().await.unwrap();
    let object = client
        .get(format!("{v3}/generations/{generation}/objects/{digest}"))
        .send()
        .await
        .unwrap();
    let object_status = object.status();
    let object_bytes = object.bytes().await.unwrap();
    let old_status = client
        .get(format!("{}/catalog", base.replace("/v3", "/v2")))
        .send()
        .await
        .unwrap()
        .status();
    server.stop(true).await;
    assert_eq!(status.as_u16(), 200);
    assert_eq!(object_status.as_u16(), 200);
    assert_eq!(object_bytes.as_ref(), b"abc");
    assert_ne!(
        old_status.as_u16(),
        200,
        "v2 delivery must not remain available"
    );
    let json: Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(json["protocol"], 3);
    assert_eq!(json["generation"], generation);
    assert_eq!(generation.len(), 32);
    let item = &json["groups"][0]["mods"][0]["files"][0];
    assert_eq!(item["length"], 3);
    assert_eq!(item["xxh3_128"], digest);
    assert!(item.get("sha256").is_none());
    assert!(item.get("xxh3_64").is_none());
}

#[test]
fn v3_forced_rehash_does_not_silently_rebaseline_legacy_client_file() {
    let f = Fixture::new(true);
    write(&f.dirs.spt_server, PLUGIN, b"abc");
    f.db.lock()
        .insert_file(f.id, PLUGIN, Some("78af5f94892f3950"), Some(3))
        .unwrap();
    let input = f.input();
    let store = f.store(64);
    assert!(build_snapshot(&input, &store, None, PublishMode::Rehash).is_err());
    let rows = f.db.lock().get_files_for_mod(f.id).unwrap();
    assert_eq!(rows[0].file_hash.as_deref(), Some("78af5f94892f3950"));
    assert_eq!(fs::read(f.dirs.spt_server.join(PLUGIN)).unwrap(), b"abc");
}

#[test]
fn v3_refuses_untracked_bundle_payload() {
    let f = Fixture::new(true);
    write(&f.dirs.spt_server, PLUGIN, b"abc");
    f.db.lock()
        .insert_file(
            f.id,
            PLUGIN,
            Some("06b05ab6733a618578af5f94892f3950"),
            Some(3),
        )
        .unwrap();
    f.add("SPT_Runtime/user/mods/Test/index.js", b"marker");
    write(
        &f.dirs.spt_server,
        "SPT_Runtime/user/mods/Test/bundles.json",
        br#"{"manifest":[{"key":"data.bundle"}]}"#,
    );
    write(
        &f.dirs.spt_server,
        "SPT_Runtime/user/mods/Test/bundles/data.bundle",
        b"untracked bundle",
    );
    let result = build_snapshot(&f.input(), &f.store(1024), None, PublishMode::Managed);
    assert!(result.is_err(), "untracked bundle must not be published");
}

#[test]
fn v3_rehash_refuses_legacy_bundle_declaration() {
    let f = Fixture::new(true);
    write(&f.dirs.spt_server, PLUGIN, b"abc");
    f.db.lock()
        .insert_file(
            f.id,
            PLUGIN,
            Some("06b05ab6733a618578af5f94892f3950"),
            Some(3),
        )
        .unwrap();
    let declaration = "SPT_Runtime/user/mods/Test/bundles.json";
    let declaration_bytes = br#"{"manifest":[{"key":"data.bundle"}]}"#;
    write(&f.dirs.spt_server, declaration, declaration_bytes);
    f.db.lock()
        .insert_file(
            f.id,
            declaration,
            Some("78af5f94892f3950"),
            Some(declaration_bytes.len() as i64),
        )
        .unwrap();
    let bundle = "SPT_Runtime/user/mods/Test/bundles/data.bundle";
    write(&f.dirs.spt_server, bundle, b"abc");
    f.db.lock()
        .insert_file(
            f.id,
            bundle,
            Some("06b05ab6733a618578af5f94892f3950"),
            Some(3),
        )
        .unwrap();
    let result = build_snapshot(&f.input(), &f.store(1024), None, PublishMode::Rehash);
    assert!(
        result.is_err(),
        "legacy bundle declaration must not be rebaselined implicitly"
    );
}

#[test]
fn v3_rejects_pending_row_even_if_hash_looks_current() {
    let f = Fixture::new(true);
    write(&f.dirs.spt_server, PLUGIN, b"abc");
    let db = f.db.lock();
    let id = db
        .insert_file(
            f.id,
            PLUGIN,
            Some("06b05ab6733a618578af5f94892f3950"),
            Some(3),
        )
        .unwrap();
    db.conn()
        .execute(
            "INSERT INTO pending_file_rehashes (file_id) VALUES (?1)",
            [id],
        )
        .unwrap();
    drop(db);
    assert!(build_snapshot(&f.input(), &f.store(64), None, PublishMode::Managed).is_err());
}

#[actix_web::test]
async fn v3_promotion_preserves_unowned_digest_named_files() {
    let f = Fixture::new(true);
    f.add(PLUGIN, b"abc");
    let store = f.dirs.convoy_store_dir();
    let _ = f.store(128);
    let unrelated_digest = format!(
        "{:032x}",
        xxhash_rust::xxh3::xxh3_128(b"not a Convoy object")
    );
    let unrelated = store.join(unrelated_digest);
    fs::write(&unrelated, b"not a Convoy object").unwrap();
    let publisher = f.publisher(128);
    publication::ready(&publisher).await;
    assert_eq!(fs::read(unrelated).unwrap(), b"not a Convoy object");
}

#[actix_web::test]
async fn v3_retirement_preserves_a_replacement_at_an_old_object_path() {
    use std::os::unix::fs::PermissionsExt;
    const OTHER: &str = "BepInEx/plugins/Test/other.dll";
    let f = Fixture::new(true);
    f.add(PLUGIN, b"abc");
    f.add(OTHER, b"def");
    let publisher = f.publisher(128);
    let old = publication::ready(&publisher).await;
    let other_digest = format!("{:032x}", xxhash_rust::xxh3::xxh3_128(b"def"));
    let pinned_a = publisher.admit(Some(&old), Some(ABC)).await.unwrap();
    let pinned_b = publisher
        .admit(Some(&old), Some(&other_digest))
        .await
        .unwrap();
    write(&f.dirs.spt_server, PLUGIN, b"ghi");
    write(&f.dirs.spt_server, OTHER, b"ghi");
    publisher.refresh(PublishMode::Rehash);
    assert_ne!(publication::ready(&publisher).await, old);
    let store = f.dirs.convoy_store_dir();
    let replaced = store.join(ABC);
    fs::remove_file(&replaced).unwrap();
    fs::write(&replaced, b"user replacement").unwrap();
    fs::set_permissions(&replaced, fs::Permissions::from_mode(0o400)).unwrap();
    drop(pinned_a);
    drop(pinned_b);
    tokio::time::timeout(std::time::Duration::from_secs(5), async {
        while store.join(&other_digest).exists() {
            tokio::time::sleep(std::time::Duration::from_millis(20)).await;
        }
    })
    .await
    .expect("retirement did not run for the control object");
    assert_eq!(fs::read(replaced).unwrap(), b"user replacement");
}

#[actix_web::test]
async fn v2_external_commit_fences_admission() {
    let f = Fixture::new(true);
    let payload = vec![b'x'; 16 * 1024 * 1024];
    let digest = format!("{:032x}", xxhash_rust::xxh3::xxh3_128(&payload));
    f.add(PLUGIN, &payload);
    const ADDON: &str = "BepInEx/plugins/Test/Addon.dll";
    write(&f.dirs.spt_server, ADDON, b"abc");
    let cold = std::time::Instant::now();
    let publisher = Arc::new(f.publisher(48 * 1024 * 1024));
    let generation = publication::ready(&publisher).await;
    println!("cold start to ready: {:?}", cold.elapsed());
    let (base, server) = serve(publisher.clone()).await;
    let client = reqwest::Client::builder()
        .no_proxy()
        .timeout(std::time::Duration::from_secs(10))
        .build()
        .unwrap();
    let catalog = format!("{base}/catalog");
    let url = format!("{base}/generations/{generation}/objects/{digest}");
    let ready_request = std::time::Instant::now();
    let mut admitted = client.get(&url).send().await.unwrap();
    assert_eq!(admitted.status().as_u16(), 200);
    let first = admitted.chunk().await.unwrap().unwrap();
    println!(
        "ready request to first body bytes: {:?}",
        ready_request.elapsed()
    );
    assert!(first.len() < payload.len());
    assert_eq!(&first[..], &payload[..first.len()]);

    let writer = Database::open(&f.dirs.db_path()).unwrap();
    let (release_app, app_held) = {
        let app = f.db.clone();
        let (held, app_held) = tokio::sync::oneshot::channel();
        let (release, wait) = std::sync::mpsc::channel::<()>();
        std::thread::spawn(move || {
            let _guard = app.lock();
            held.send(()).unwrap();
            let _ = wait.recv();
        });
        (release, app_held)
    };
    app_held.await.unwrap();
    let before = writer.convoy_revision().unwrap();
    writer
        .insert_sync_report("aid", "up_to_date", None, None, None, None)
        .unwrap();
    assert_eq!(writer.convoy_revision().unwrap(), before);
    {
        let tx = writer.begin_transaction().unwrap();
        tx.execute(
            "UPDATE installed_mods SET name = 'Rolled back' WHERE id = ?1",
            [f.id],
        )
        .unwrap();
        assert_eq!(
            client
                .get(&catalog)
                .header("if-none-match", format!("\"{generation}\""))
                .send()
                .await
                .unwrap()
                .status()
                .as_u16(),
            304
        );
        tx.rollback().unwrap();
    }
    assert_eq!(writer.convoy_revision().unwrap(), before);
    let tx = writer.begin_transaction().unwrap();
    tx.execute(
        "UPDATE installed_mods SET name = 'Renamed' WHERE id = ?1",
        [f.id],
    )
    .unwrap();
    let addon_id = writer
        .insert_addon(1, f.id, 1, "Fixture addon", None, "1", None)
        .unwrap();
    writer
        .insert_addon_file(addon_id, ADDON, Some(ABC), Some(3))
        .unwrap();
    tx.commit().unwrap();
    let status = client.get(&url).send().await.unwrap().status().as_u16();
    assert_eq!(
        &admitted.bytes().await.unwrap()[..],
        &payload[first.len()..]
    );
    publication::wait_for(
        &publisher,
        |s| matches!(s, PublicationStatus::Ready { generation: new, .. } if new != &generation),
    )
    .await;
    let current = client.get(&catalog).send().await.unwrap();
    assert_eq!(current.status().as_u16(), 200);
    let manifest: Value = current.json().await.unwrap();
    assert_eq!(manifest["groups"][0]["mods"][0]["files"][0]["path"], ADDON);
    release_app.send(()).unwrap();
    server.stop(false).await;
    assert!(
        matches!(status, 409 | 503),
        "retired generation returned {status}"
    );
}

// Observe source content reads without instrumenting the publisher.
struct SourceReads(fs::File);

impl SourceReads {
    fn watch(path: &Path) -> Self {
        use std::{
            ffi::CString,
            os::{fd::FromRawFd, unix::ffi::OsStrExt},
        };
        let path = CString::new(path.as_os_str().as_bytes()).unwrap();
        // SAFETY: successful initialization transfers ownership of a new FD.
        let fd = unsafe { libc::inotify_init1(libc::IN_NONBLOCK | libc::IN_CLOEXEC) };
        assert!(fd >= 0);
        let file = unsafe { fs::File::from_raw_fd(fd) };
        // SAFETY: the CString and owned descriptor remain live for this call.
        assert!(
            unsafe {
                libc::inotify_add_watch(fd, path.as_ptr(), libc::IN_ACCESS | libc::IN_CLOSE_NOWRITE)
            } >= 0
        );
        Self(file)
    }

    fn drain(&mut self) -> usize {
        use std::io::{ErrorKind, Read};
        let mut count = 0;
        let mut buf = [0u8; 4096];
        loop {
            match self.0.read(&mut buf) {
                Ok(n) if n != 0 => {
                    let (events, remainder) =
                        buf[..n].as_chunks::<{ std::mem::size_of::<libc::inotify_event>() }>();
                    assert!(remainder.is_empty());
                    for event in events {
                        let mask = u32::from_ne_bytes(event[4..8].try_into().unwrap());
                        assert_eq!(u32::from_ne_bytes(event[12..16].try_into().unwrap()), 0);
                        assert_eq!(mask & libc::IN_Q_OVERFLOW, 0);
                        count += usize::from(mask & libc::IN_ACCESS != 0);
                    }
                }
                Err(err) if err.kind() == ErrorKind::WouldBlock => return count,
                other => panic!("unexpected inotify read: {other:?}"),
            }
        }
    }
}

#[actix_web::test]
async fn v2_reuses_unaffected_objects() {
    let f = Fixture::new(true);
    f.add(PLUGIN, b"abc");
    const OTHER: &str = "BepInEx/plugins/Other/other.dll";
    let other =
        f.db.lock()
            .insert_mod(None, None, "Other", None, "1", "local", None, None)
            .unwrap();
    write(&f.dirs.spt_server, OTHER, b"unaffected");
    f.track(other, OTHER, b"unaffected");
    let publisher = f.publisher(64);
    let original = publication::ready(&publisher).await;
    let mut changed_reads = SourceReads::watch(&f.dirs.spt_server.join(PLUGIN));
    let mut unaffected_reads = SourceReads::watch(&f.dirs.spt_server.join(OTHER));
    f.db.lock()
        .conn()
        .execute(
            "UPDATE installed_mods SET name = 'Renamed' WHERE id = ?1",
            [f.id],
        )
        .unwrap();
    publisher.refresh(PublishMode::Managed);
    let renamed = publication::ready(&publisher).await;
    assert_ne!(renamed, original);
    let metadata_reads = (changed_reads.drain(), unaffected_reads.drain());
    assert_eq!(metadata_reads, (0, 0));

    write(&f.dirs.spt_server, PLUGIN, b"def");
    let hash = format!("{:032x}", xxhash_rust::xxh3::xxh3_128(b"def"));
    f.db.lock()
        .conn()
        .execute(
            "UPDATE installed_files SET file_hash = ?1 WHERE file_path = ?2",
            rusqlite::params![hash, PLUGIN],
        )
        .unwrap();
    publisher.refresh(PublishMode::Managed);
    assert_ne!(publication::ready(&publisher).await, renamed);
    let content_reads = (changed_reads.drain(), unaffected_reads.drain());
    println!("source access events (changed, unaffected): metadata={metadata_reads:?} payload update={content_reads:?}");
    assert!(content_reads.0 > 0);
    assert_eq!(content_reads.1, 0);
}

#[actix_web::test]
async fn v2_rehash_rebaselines_the_published_bytes() {
    let f = Fixture::new(true);
    f.add(PLUGIN, b"abc");
    let publisher = f.publisher(64);
    let original = publication::ready(&publisher).await;
    write(&f.dirs.spt_server, PLUGIN, b"new bytes");
    publisher.refresh(PublishMode::Rehash);
    publisher.refresh(PublishMode::Managed);
    assert_ne!(publication::ready(&publisher).await, original);
    let (hash, length): (String, i64) =
        f.db.lock()
            .conn()
            .query_row(
                "SELECT file_hash, file_size FROM installed_files WHERE file_path = ?1",
                [PLUGIN],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .unwrap();
    assert_eq!(
        hash,
        format!("{:032x}", xxhash_rust::xxh3::xxh3_128(b"new bytes"))
    );
    assert_eq!(length, 9);
}

#[actix_web::test]
async fn v2_rejects_mandatory_config_directory_asset() {
    let f = Fixture::new(true);
    f.add(
        "BepInEx/config",
        b"cannot occupy a mandatory client directory",
    );
    let publisher = f.publisher(64);
    publication::wait_for(&publisher, |status| {
        matches!(status, PublicationStatus::Failed { .. })
    })
    .await;
}

#[actix_web::test]
async fn v2_restart_reuses_the_verified_receipt_without_rereading_sources() {
    let f = Fixture::new(true);
    f.add(PLUGIN, b"abc");
    let original = {
        let publisher = f.publisher(64);
        publication::ready(&publisher).await
    };
    fs::remove_file(f.dirs.spt_server.join(PLUGIN)).unwrap();
    let publisher = f.publisher(64);
    assert_eq!(publication::ready(&publisher).await, original);
}

#[actix_web::test]
async fn v2_startup_detects_same_length_cache_corruption() {
    use std::os::unix::fs::PermissionsExt;
    let f = Fixture::new(true);
    f.add(PLUGIN, b"abc");
    let generation = {
        let publisher = f.publisher(64);
        publication::ready(&publisher).await
    };
    let path = f.dirs.convoy_store_dir().join(ABC);
    fs::set_permissions(&path, fs::Permissions::from_mode(0o600)).unwrap();
    fs::write(&path, b"bad").unwrap();
    fs::set_permissions(&path, fs::Permissions::from_mode(0o400)).unwrap();
    let publisher = Arc::new(f.publisher(64));
    let (base, server) = serve(publisher.clone()).await;
    let client = reqwest::Client::builder().no_proxy().build().unwrap();
    let object = client
        .get(format!("{base}/generations/{generation}/objects/{ABC}"))
        .send()
        .await
        .unwrap();
    match object.status().as_u16() {
        200 => assert_eq!(&object.bytes().await.unwrap()[..], b"abc"),
        503 => assert!(matches!(
            object.json::<Value>().await.unwrap()["code"].as_str(),
            Some("publish_failed" | "preparing")
        )),
        status => panic!("unexpected response {status}"),
    }
    server.stop(false).await;
}

#[actix_web::test]
async fn v2_leases_are_bounded_and_pin_only_live_objects() {
    use spt_quartermaster::convoy::publisher::DeliveryError;
    let f = Fixture::new(true);
    f.add(PLUGIN, b"abc");
    let publisher = f.publisher(7);
    let generation = publication::ready(&publisher).await;
    let mut leases = Vec::new();
    for _ in 0..16 {
        leases.push(publisher.admit(Some(&generation), Some(ABC)).await.unwrap());
    }
    assert!(matches!(
        publisher.admit(Some(&generation), Some(ABC)).await,
        Err(DeliveryError::Busy)
    ));
    let replace = |bytes: &[u8]| {
        f.db.lock().delete_files_for_mod(f.id).unwrap();
        f.add(PLUGIN, bytes);
        publisher.refresh(PublishMode::Managed);
    };
    replace(b"def");
    assert_ne!(publication::ready(&publisher).await, generation);
    assert!(f.dirs.convoy_store_dir().join(ABC).is_file());
    replace(b"ghi");
    publication::wait_for(&publisher, |s| {
        matches!(s, PublicationStatus::Failed { .. })
    })
    .await;
    drop(leases);
    publication::wait_for(&publisher, |s| matches!(s, PublicationStatus::Ready { .. })).await;
    assert!(!f.dirs.convoy_store_dir().join(ABC).exists());
}

#[actix_web::test]
async fn v2_restart_never_confuses_equal_revisions_with_equal_inventory() {
    let old = Fixture::new(true);
    old.add(PLUGIN, b"abc");
    let original = {
        let p = old.publisher(64);
        publication::ready(&p).await
    };
    let (revision, id, data): (i64, String, Vec<u8>) = old
        .db
        .lock()
        .conn()
        .query_row(
            "SELECT revision, receipt, receipt_data FROM convoy_publication",
            [],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
        )
        .unwrap();
    let restored = Fixture::new(true);
    restored.add(PLUGIN, b"def");
    assert_eq!(restored.db.lock().convoy_revision().unwrap(), revision);
    restored
        .db
        .lock()
        .conn()
        .execute(
            "UPDATE convoy_publication SET receipt = ?1, receipt_data = ?2, receipt_revision = ?3",
            rusqlite::params![id, data, revision],
        )
        .unwrap();
    let _store = restored.store(64);
    fs::copy(
        old.dirs.convoy_store_dir().join(ABC),
        restored.dirs.convoy_store_dir().join(ABC),
    )
    .unwrap();
    let publisher = restored.publisher(64);
    assert_ne!(publication::ready(&publisher).await, original);
    // A receipt from another inventory is not proof of ownership after restart.
    assert_eq!(
        fs::read(restored.dirs.convoy_store_dir().join(ABC)).unwrap(),
        b"abc"
    );
}

#[actix_web::test]
async fn v2_policy_change_fences_the_warm_store_during_preparation() {
    use spt_quartermaster::convoy::publisher::DeliveryError;
    use std::os::fd::AsRawFd;
    let f = Fixture::new(true);
    f.add(PLUGIN, b"abc");
    let publisher = f.publisher(64);
    let old = publication::ready(&publisher).await;
    let store_guard = fs::File::open(f.dirs.convoy_store_dir()).unwrap();
    // SAFETY: flock holds a lock on the open directory until the guard closes.
    assert_eq!(
        unsafe { libc::flock(store_guard.as_raw_fd(), libc::LOCK_EX) },
        0
    );
    f.config.write().convoy.as_mut().unwrap().enabled = false;
    publisher.refresh(PublishMode::Managed);
    assert!(matches!(
        publisher.admit(Some(&old), Some(ABC)).await,
        Err(DeliveryError::Disabled)
    ));
    {
        let mut config = f.config.write();
        let convoy = config.convoy.as_mut().unwrap();
        convoy.enabled = true;
        convoy
            .exclusions
            .push("BepInEx/plugins/Test/user.json".into());
    }
    publisher.refresh(PublishMode::Managed);
    assert!(matches!(
        publisher.admit(Some(&old), Some(ABC)).await,
        Err(DeliveryError::Preparing)
    ));
    drop(store_guard);
    assert_ne!(publication::ready(&publisher).await, old);
    assert!(matches!(
        publisher.admit(Some(&old), Some(ABC)).await,
        Err(DeliveryError::StaleGeneration)
    ));
}

#[actix_web::test]
async fn v2_rehash_uses_and_rebaselines_the_same_bundle_declaration() {
    const DECLARATION: &str = "SPT_Runtime/user/mods/Test/bundles.json";
    const SOURCE: &str = "SPT_Runtime/user/mods/Test/bundles/a.bundle";
    let fixture = Fixture::new(true);
    fixture.add(PLUGIN, b"abc");
    fixture.add(DECLARATION, br#"{"manifest":[{"key":"a.bundle"}]}"#);
    fixture.add(SOURCE, b"old bundle");
    let publisher = fixture.publisher(128);
    let initial = publication::ready(&publisher).await;
    let declaration = br#" { "manifest" : [ { "key" : "a.bundle" } ] } "#;
    write(&fixture.dirs.spt_server, DECLARATION, declaration);
    write(&fixture.dirs.spt_server, SOURCE, b"changed bundle");
    fixture
        .db
        .lock()
        .conn()
        .execute("UPDATE installed_mods SET name = 'Renamed'", [])
        .unwrap();
    publisher.refresh(PublishMode::Managed);
    let renamed = publication::ready(&publisher).await;
    assert_ne!(renamed, initial);
    let asset = |json: &web::Bytes| -> Value {
        let catalog: Value = serde_json::from_slice(json).unwrap();
        catalog["groups"][0]["mods"][0]["bundles"][0].clone()
    };
    let spt_quartermaster::convoy::publisher::Admission::Manifest { json, .. } =
        publisher.admit(None, None).await.unwrap()
    else {
        panic!("expected manifest")
    };
    assert_eq!(asset(&json)["length"], 10);
    publisher.refresh(PublishMode::Rehash);
    assert_ne!(publication::ready(&publisher).await, renamed);
    let spt_quartermaster::convoy::publisher::Admission::Manifest { json, .. } =
        publisher.admit(None, None).await.unwrap()
    else {
        panic!("expected manifest")
    };
    assert_eq!(asset(&json)["length"], 14);
    for (path, expected) in [
        (DECLARATION, declaration.as_slice()),
        (SOURCE, b"changed bundle".as_slice()),
    ] {
        let file: (String, i64) = fixture
            .db
            .lock()
            .conn()
            .query_row(
                "SELECT file_hash, file_size FROM installed_files WHERE file_path = ?1",
                [path],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .unwrap();
        assert_eq!(
            file,
            (
                format!("{:032x}", xxhash_rust::xxh3::xxh3_128(expected)),
                expected.len() as i64
            )
        );
    }
}

#[actix_web::test]
async fn v2_restart_reuses_packages_after_offline_metadata_change() {
    let f = Fixture::new(true);
    f.add(PLUGIN, b"abc");
    let original = {
        let p = f.publisher(64);
        publication::ready(&p).await
    };
    fs::remove_file(f.dirs.spt_server.join(PLUGIN)).unwrap();
    f.db.lock()
        .conn()
        .execute("UPDATE installed_mods SET name = 'Renamed offline'", [])
        .unwrap();
    let publisher = f.publisher(64);
    assert_ne!(publication::ready(&publisher).await, original);
}

fn no_zip(root: &Path) {
    for entry in root.read_dir().unwrap() {
        let entry = entry.unwrap();
        if entry.file_type().unwrap().is_dir() {
            no_zip(&entry.path());
        } else {
            let name = entry.file_name();
            let name = name.to_string_lossy();
            assert!(!(name.starts_with("convoy-") && name.ends_with(".zip")));
        }
    }
}

#[actix_web::test]
async fn v2_published_file_and_ranges() {
    let f = Fixture::new(true);
    f.add(PLUGIN, b"abc");
    f.add("BepInEx/plugins/Test/empty", b"");
    let publisher = Arc::new(f.publisher(3));
    publication::ready(&publisher).await;
    let (base, server) = serve(publisher.clone()).await;
    let client = reqwest::Client::builder().no_proxy().build().unwrap();
    let catalog = format!("{base}/catalog");
    let res = client.get(&catalog).send().await.unwrap();
    assert_eq!(res.status().as_u16(), 200);
    let etag = res.headers()["etag"].to_str().unwrap().to_owned();
    assert_eq!(res.headers()["cache-control"], "no-cache");
    let wire = res.text().await.unwrap();
    println!("catalog fixture: {wire}");
    let json: Value = serde_json::from_str(&wire).unwrap();
    let generation = json["generation"].as_str().unwrap();
    assert_eq!(json["protocol"], 3);
    assert_eq!(etag, format!("\"{generation}\""));
    let item = &json["groups"][0]["mods"][0]["files"][1];
    assert_eq!(item["path"], PLUGIN);
    assert_eq!(item["length"], 3);
    assert_eq!(item["xxh3_128"], ABC);
    assert!(item.get("sha256").is_none());
    assert!(item.get("xxh3_64").is_none());
    let object = format!("{base}/generations/{generation}/objects/{ABC}");
    let validator = format!("\"{ABC}\"");
    // Source disappears and SQLite remains write-locked during raw delivery.
    fs::remove_file(f.dirs.spt_server.join(PLUGIN)).unwrap();
    let conn = rusqlite::Connection::open(f.dirs.db_path()).unwrap();
    conn.execute_batch("BEGIN IMMEDIATE").unwrap();
    let res = client
        .get(&object)
        .header("accept-encoding", "gzip, br")
        .timeout(std::time::Duration::from_secs(2))
        .send()
        .await
        .unwrap();
    assert_eq!(res.status().as_u16(), 200);
    assert_eq!(res.headers()["accept-ranges"], "bytes");
    assert_eq!(res.headers()["etag"], validator);
    assert_eq!(res.headers()["content-length"], "3");
    assert_eq!(res.headers()["cache-control"], "no-store");
    assert!(res.headers().get("content-encoding").is_none());
    println!("object 200 fixture: {:?}", res.headers());
    assert_eq!(res.bytes().await.unwrap().as_ref(), b"abc");
    conn.execute_batch("ROLLBACK").unwrap();
    for (range, if_range, status, content_range, bytes) in [
        (
            "bytes=1-",
            Some(validator.as_str()),
            206,
            Some("bytes 1-2/3"),
            &b"bc"[..],
        ),
        ("bytes=1-", Some("\"other\""), 200, None, &b"abc"[..]),
        ("bytes=1-", Some("W/\"other\""), 200, None, &b"abc"[..]),
        ("bytes=1-", None, 200, None, &b"abc"[..]),
        (
            "bytes=3-",
            Some(validator.as_str()),
            416,
            Some("bytes */3"),
            &b""[..],
        ),
        (
            "bytes=1-2,3-",
            Some(validator.as_str()),
            416,
            Some("bytes */3"),
            &b""[..],
        ),
    ] {
        let mut req = client.get(&object).header("range", range);
        if let Some(v) = if_range {
            req = req.header("if-range", v);
        }
        let res = req.send().await.unwrap();
        assert_eq!(res.status().as_u16(), status, "{range} {if_range:?}");
        assert_eq!(
            res.headers()
                .get("content-range")
                .map(|v| v.to_str().unwrap()),
            content_range
        );
        assert_eq!(res.headers()["content-length"], bytes.len().to_string());
        if matches!(status, 206 | 416) {
            println!("object {status} fixture: {:?}", res.headers());
        }
        assert_eq!(res.bytes().await.unwrap().as_ref(), bytes);
    }
    let res = client.get(object.replace(ABC, EMPTY)).send().await.unwrap();
    assert_eq!(res.status().as_u16(), 200);
    assert_eq!(res.headers()["content-length"], "0");
    assert!(res.bytes().await.unwrap().is_empty());
    let res = client
        .get(object.replace(ABC, &"0".repeat(32)))
        .send()
        .await
        .unwrap();
    assert_eq!(res.status().as_u16(), 404);
    assert_eq!(res.json::<Value>().await.unwrap()["code"], "not_published");
    let res = client
        .get(object.replace(generation, &"0".repeat(32)))
        .send()
        .await
        .unwrap();
    assert_eq!(res.status().as_u16(), 409);
    assert_eq!(
        res.json::<Value>().await.unwrap()["code"],
        "stale_generation"
    );
    assert_eq!(
        client
            .get(&catalog)
            .header("if-none-match", &etag)
            .send()
            .await
            .unwrap()
            .status()
            .as_u16(),
        304
    );
    f.config.write().convoy.as_mut().unwrap().enabled = false;
    for url in [&catalog, &object] {
        let res = client
            .get(url)
            .header("if-none-match", &etag)
            .send()
            .await
            .unwrap();
        assert_eq!(res.status().as_u16(), 503);
        assert_eq!(res.headers()["cache-control"], "no-store");
        assert_eq!(res.json::<Value>().await.unwrap()["code"], "disabled");
    }
    f.config.write().convoy.as_mut().unwrap().enabled = true;
    publisher.refresh(PublishMode::Managed);
    publication::ready(&publisher).await;
    // A same-length replacement is still a storage failure, not a ready 200.
    let stored = f.dirs.convoy_store_dir().join(ABC);
    fs::remove_file(&stored).unwrap();
    fs::write(&stored, b"bad").unwrap();
    use std::os::unix::fs::PermissionsExt;
    fs::set_permissions(&stored, fs::Permissions::from_mode(0o400)).unwrap();
    let res = client.get(object).send().await.unwrap();
    assert_eq!(res.status().as_u16(), 503);
    assert_eq!(res.json::<Value>().await.unwrap()["code"], "publish_failed");
    no_zip(&f.dirs.root);
    server.stop(false).await;
}

#[test]
fn v2_scoped_complete_snapshot() {
    for (nested, runtime) in [(true, "SPT_Runtime"), (false, "SPT")] {
        let f = Fixture::new(nested);
        f.add(PLUGIN, if nested { b"base" } else { b"abc" });
        if nested {
            write(&f.dirs.mod_overlay(), PLUGIN, b"overlay");
            write(&f.dirs.runtime_upper(), PLUGIN, b"abc");
            f.db.lock().delete_files_for_mod(f.id).unwrap();
            f.track(f.id, PLUGIN, b"abc");
        }
        f.add(MANAGED, b"abc");
        f.add("EscapeFromTarkov_Data/Managed/sub/No.dll", b"server");
        f.add("BepInEx/CONFIG/cONVOY/private", b"secret");
        f.add(&format!("{runtime}/user/mods/Test/server.dll"), b"server");
        let bundle = format!("{runtime}/user/mods/Test/bundles/test.bundle");
        let mod_root = if nested {
            f.dirs.mod_overlay()
        } else {
            f.dirs.spt_server.clone()
        };
        write(&mod_root, &bundle, b"abc");
        f.track(f.id, &bundle, b"abc");
        let declaration = format!("{runtime}/user/mods/Test/bundles.json");
        write(
            &mod_root,
            &declaration,
            br#"{"manifest":[{"key":"test.bundle"}]}"#,
        );
        write(
            &mod_root,
            &format!("{runtime}/user/mods/Test/package.json"),
            br#"{"name":"fixture.guid"}"#,
        );
        let store = f.store(3);
        let input = f.input();
        let snapshot = build_snapshot(&input, &store, None, PublishMode::Managed).unwrap();
        let m = &snapshot.manifest.groups[0].mods[0];
        assert_eq!(m.guid.as_deref(), Some("fixture.guid"));
        assert_eq!(
            m.files
                .iter()
                .map(|e| (e.path.as_str(), e.xxh3_128.as_str()))
                .collect::<Vec<_>>(),
            [(PLUGIN, ABC), (MANAGED, ABC)]
        );
        assert_eq!(m.bundles.len(), 1);
        assert_eq!(
            m.bundles[0].path,
            format!("{runtime}/user/cache/bundles/test.bundle")
        );
        assert_eq!(m.bundles[0].xxh3_128, ABC);
        let builds = || build_snapshot(&input, &store, None, PublishMode::Managed);
        fs::remove_file(mod_root.join(&bundle)).unwrap();
        assert!(builds().is_err(), "declared missing bundle");
        std::os::unix::fs::symlink(f.dirs.spt_server.join(MANAGED), mod_root.join(&bundle))
            .unwrap();
        assert!(
            builds().is_err(),
            "declared bundle escaping its own bundle scope"
        );
        fs::remove_file(mod_root.join(&declaration)).unwrap();
        assert!(builds().unwrap().manifest.groups[0].mods[0]
            .bundles
            .is_empty());
        let source = if nested {
            f.dirs.runtime_upper()
        } else {
            f.dirs.spt_server.clone()
        };
        fs::remove_file(source.join(PLUGIN)).unwrap();
        if nested {
            write(&source, "BepInEx/plugins/Test/.wh.item.dll", b"");
            assert!(builds().is_err(), "masked lower source");
        } else {
            assert!(builds().is_err(), "missing client source");
        }
        let outside = tempfile::NamedTempFile::new().unwrap();
        fs::write(outside.path(), b"abc").unwrap();
        std::os::unix::fs::symlink(outside.path(), source.join(PLUGIN)).unwrap();
        assert!(
            builds().is_err(),
            "escaping client source with matching contents"
        );
    }
}

#[test]
fn v2_rejects_parent_file_alias() {
    let f = Fixture::new(false);
    f.add("BepInEx/plugins/Item.dll", b"abc");
    let optional =
        f.db.lock()
            .insert_group("Optional", "optional", "optional", false)
            .unwrap();
    let other =
        f.db.lock()
            .insert_mod(None, None, "Other", None, "1", "local", None, None)
            .unwrap();
    f.db.lock().set_mod_group(other, Some(optional)).unwrap();
    let child = "BepInEx/plugins/item.dll/child";
    write(&f.dirs.spt_server, child, b"abc");
    f.track(other, child, b"abc");
    let err = build_snapshot(&f.input(), &f.store(3), None, PublishMode::Managed).unwrap_err();
    assert!(
        err.to_string().contains("alias") || err.to_string().contains("collision"),
        "{err:#}"
    );
}

#[actix_web::test]
async fn v2_budget_does_not_advertise_an_incomplete_candidate() {
    let f = Fixture::new(false);
    f.add(PLUGIN, b"abc");
    let publisher = Arc::new(f.publisher(4));
    let generation = publication::ready(&publisher).await;
    let (base, server) = serve(publisher.clone()).await;
    f.add("BepInEx/plugins/Test/a-first.dll", b"x");
    f.add("BepInEx/plugins/Test/new.dll", b"ef");
    publisher.refresh(PublishMode::Managed);
    publication::wait_for(&publisher, |s| {
        matches!(s, PublicationStatus::Failed { .. })
    })
    .await;
    assert_eq!(fs::read_dir(f.dirs.convoy_store_dir()).unwrap().count(), 1);
    let client = reqwest::Client::builder().no_proxy().build().unwrap();
    let response = client
        .get(format!("{base}/generations/{generation}/objects/{ABC}"))
        .send()
        .await
        .unwrap();
    assert_eq!(response.status().as_u16(), 503);
    server.stop(false).await;
}

#[test]
fn v2_rejects_confined_client_aliases() {
    for target in ["BepInEx/plugins/untracked.dll", MANAGED] {
        let f = Fixture::new(false);
        f.add(PLUGIN, b"abc");
        write(&f.dirs.spt_server, target, b"abc");
        fs::remove_file(f.dirs.spt_server.join(PLUGIN)).unwrap();
        std::os::unix::fs::symlink(
            f.dirs.spt_server.join(target),
            f.dirs.spt_server.join(PLUGIN),
        )
        .unwrap();
        assert!(
            build_snapshot(&f.input(), &f.store(8), None, PublishMode::Managed).is_err(),
            "borrowed {target}"
        );
    }
}

#[test]
fn v2_rejects_changed_tracked_bundle_declaration() {
    const DECL: &str = "SPT/user/mods/Test/bundles.json";
    let f = Fixture::new(false);
    f.add(PLUGIN, b"abc");
    f.add(DECL, br#"{"manifest":[{"key":"a"}]}"#);
    write(&f.dirs.spt_server, "SPT/user/mods/Test/bundles/a", b"abc");
    write(&f.dirs.spt_server, "SPT/user/mods/Test/bundles/b", b"abc");
    f.track(f.id, "SPT/user/mods/Test/bundles/a", b"abc");
    f.track(f.id, "SPT/user/mods/Test/bundles/b", b"abc");
    let store = f.store(8);
    let input = f.input();
    assert!(build_snapshot(&input, &store, None, PublishMode::Managed).is_ok());
    for changed in [
        br#"{"manifest":[{"key":"b"}]}"#.as_slice(),
        br#"{"manifest":[]}"#,
    ] {
        write(&f.dirs.spt_server, DECL, changed);
        assert!(build_snapshot(&input, &store, None, PublishMode::Managed).is_err());
    }
}

#[test]
fn v2_rejects_bundle_alias_inside_subtree() {
    let f = Fixture::new(false);
    f.add(PLUGIN, b"abc");
    f.add(
        "SPT/user/mods/Test/bundles.json",
        br#"{"manifest":[{"key":"a"}]}"#,
    );
    let dir = f.dirs.spt_server.join("SPT/user/mods/Test/bundles");
    write(&dir, "other", b"abc");
    f.track(f.id, "SPT/user/mods/Test/bundles/a", b"abc");
    std::os::unix::fs::symlink("other", dir.join("a")).unwrap();
    assert!(build_snapshot(&f.input(), &f.store(8), None, PublishMode::Managed).is_err());
}

#[test]
fn v2_rejects_optional_namespace_case_aliases() {
    for (canonical, alias) in [
        (PLUGIN, "bepinex/plugins/Test/item.dll"),
        (MANAGED, "EscapeFromTarkov_Data/managed/Dependency.DLL"),
    ] {
        let f = Fixture::new(false);
        f.add(canonical, b"abc");
        let group =
            f.db.lock()
                .insert_group("Optional", "optional", "optional", false)
                .unwrap();
        let mod_id =
            f.db.lock()
                .insert_mod(None, None, "Other", None, "1", "local", None, None)
                .unwrap();
        f.db.lock().set_mod_group(mod_id, Some(group)).unwrap();
        write(&f.dirs.spt_server, alias, b"abc");
        f.track(mod_id, alias, b"abc");
        assert!(
            build_snapshot(&f.input(), &f.store(8), None, PublishMode::Managed).is_err(),
            "discarded {alias}"
        );
    }
}
