use super::*;
use std::{path::PathBuf, sync::mpsc, time::Duration};

type Gate = (mpsc::SyncSender<()>, mpsc::Receiver<()>);
static GATES: parking_lot::Mutex<BTreeMap<PathBuf, Gate>> =
    parking_lot::Mutex::new(BTreeMap::new());

pub(super) fn arm(key: impl Into<PathBuf>) -> (mpsc::Receiver<()>, mpsc::SyncSender<()>) {
    let (entered, wait) = mpsc::sync_channel(1);
    let (go, resume) = mpsc::sync_channel(1);
    assert!(GATES.lock().insert(key.into(), (entered, resume)).is_none());
    (wait, go)
}

pub(in crate::convoy) fn pause(key: &Path) {
    let gate = GATES.lock().remove(key);
    if let Some((entered, resume)) = gate {
        if entered.send(()).is_ok() {
            let _ = resume.recv_timeout(Duration::from_secs(10));
        }
    }
}

#[tokio::test]
async fn wave_authoritative_guid_skips_unusable_fallback_and_rediscovers_on_removal() -> Result<()>
{
    guid_authority_roundtrip(false).await?;
    guid_authority_roundtrip(true).await
}

async fn guid_authority_roundtrip(cold: bool) -> Result<()> {
    const PLUGIN: &str = "BepInEx/plugins/wave/item.dll";
    const SERVER: &str = "SPT_Runtime/user/mods/wave/server.dll";
    const PACKAGE: &str = "SPT_Runtime/user/mods/wave/package.json";
    let f = Fixture::new();
    f.track(PLUGIN, b"trusted plugin");
    f.track(SERVER, b"server");
    f.write(PACKAGE, br#"{"name":"obsolete-fallback"}"#);
    let guid = |value: Option<&str>| -> rusqlite::Result<usize> {
        f.db.lock().conn().execute(
            "UPDATE installed_mods SET guid = ?1 WHERE id = ?2",
            rusqlite::params![value, f.id],
        )
    };
    let mut p = f.publisher();
    ready(&p).await;
    guid(Some("database-guid"))?;
    let package = f.dirs.spt_server.join(PACKAGE);
    let alias = package.with_extension("alias");
    std::fs::hard_link(&package, &alias)?;
    // Force fresh builds with the bad fallback, not just warm reuse.
    p.refresh(PublishMode::Rehash);
    ready(&p).await;
    let read_guid = |admission| -> Result<String> {
        let Admission::Manifest { json, .. } = admission else {
            anyhow::bail!("expected catalog")
        };
        let catalog: CatalogManifest = serde_json::from_slice(&json)?;
        catalog.groups[0].mods[0]
            .guid
            .clone()
            .context("expected GUID")
    };
    assert_eq!(
        read_guid(p.admit(None, None).await.expect("authoritative catalog"))?,
        "database-guid"
    );
    std::fs::remove_file(alias)?;
    f.write(PACKAGE, br#"{"name":"current-fallback"}"#);
    // A metadata-only change should not re-read the warm payload source.
    std::fs::remove_file(f.dirs.spt_server.join(PLUGIN))?;
    guid(None)?;
    if cold {
        drop(p);
        p = f.publisher();
    } else {
        p.refresh(PublishMode::Managed);
    }
    let generation = ready(&p).await;
    assert_eq!(
        read_guid(p.admit(None, None).await.expect("fallback catalog"))?,
        "current-fallback"
    );
    assert!(matches!(
        p.admit(Some(&generation), Some(&receipt::digest(b"trusted plugin")))
            .await,
        Ok(Admission::Object { .. })
    ));
    Ok(())
}

pub(super) struct Fixture {
    _root: tempfile::TempDir,
    pub(super) dirs: QumaDirs,
    pub(super) config: Arc<RwLock<Config>>,
    pub(super) db: Arc<parking_lot::Mutex<Database>>,
    id: i64,
}

impl Fixture {
    pub(super) fn new() -> Self {
        let root = tempfile::tempdir().expect("create isolated fixture root");
        let dirs = QumaDirs::from_root(root.path().into());
        std::fs::create_dir_all(&dirs.spt_server).expect("create fixture directories");
        let db = Database::open(&dirs.db_path()).expect("open fixture file or database");
        let id = db
            .insert_mod(None, None, "Fixture", None, "1", "local", None, None)
            .expect("fixture operation must succeed");
        Self {
            _root: root,
            dirs,
            config: Arc::new(RwLock::new(Config {
                convoy: Some(ConvoyConfig::default()),
                ..Config::default()
            })),
            db: Arc::new(parking_lot::Mutex::new(db)),
            id,
        }
    }

    pub(super) fn write(&self, path: &str, bytes: &[u8]) {
        let path = self.dirs.spt_server.join(path);
        std::fs::create_dir_all(path.parent().expect("fixture file must have a parent"))
            .expect("fixture file must have a parent");
        std::fs::write(path, bytes).expect("write fixture bytes");
    }

    pub(super) fn track(&self, path: &str, bytes: &[u8]) {
        self.write(path, bytes);
        self.db
            .lock()
            .insert_file(self.id, path, Some(&hash(bytes)), Some(bytes.len() as i64))
            .expect("fixture operation must succeed");
    }

    pub(super) fn publisher(&self) -> Publisher {
        Publisher::start(self.dirs.clone(), self.config.clone(), self.db.clone())
            .expect("start disposable publisher")
    }
}

fn hash(bytes: &[u8]) -> String {
    format!("{:016x}", xxhash_rust::xxh3::xxh3_64(bytes))
}

async fn wait(p: &Publisher, predicate: impl Fn(&PublicationStatus) -> bool) -> PublicationStatus {
    let mut status = p.subscribe();
    tokio::time::timeout(Duration::from_secs(10), async {
        status
            .wait_for(predicate)
            .await
            .expect("publisher status channel must remain live")
            .clone()
    })
    .await
    .expect("publication did not reach completion")
}

pub(super) async fn ready(p: &Publisher) -> String {
    match wait(p, |s| {
        matches!(
            s,
            PublicationStatus::Ready { .. } | PublicationStatus::Failed { .. }
        )
    })
    .await
    {
        PublicationStatus::Ready { generation, .. } => generation,
        s => panic!("not ready: {s:?}"),
    }
}

#[tokio::test]
async fn v2_rehash_vetoes_earlier_declaration_change_during_later_copy() {
    const PLUGIN: &str = "BepInEx/plugins/witness/item.dll";
    const DECL: &str = "SPT_Runtime/user/mods/witness/bundles.json";
    const BUNDLE: &str = "SPT_Runtime/user/mods/witness/bundles/r1-witness.bundle";
    const OUTPUT: &str = "SPT_Runtime/user/cache/bundles/r1-witness.bundle";
    let f = Fixture::new();
    f.track(PLUGIN, b"original");
    f.track(DECL, br#"{"manifest":[{"key":"r1-witness.bundle"}]}"#);
    f.track(BUNDLE, &vec![b'b'; 128 * 1024]);
    let publisher = f.publisher();
    ready(&publisher).await;
    let before =
        f.db.lock()
            .convoy_stamp()
            .expect("read fixture publication stamp");
    let changed = br#" {"manifest":[{"key":"r1-witness.bundle"}]} "#;
    f.write(PLUGIN, b"new bytes");
    f.write(DECL, changed);
    let (entered, resume) = arm(OUTPUT);
    publisher.refresh(PublishMode::Rehash);
    entered
        .recv_timeout(Duration::from_secs(10))
        .expect("worker must reach barrier");
    f.write(DECL, br#"{"manifest":[]}"#);
    resume.send(()).expect("worker must accept barrier release");
    let end = wait(&publisher, |s| {
        matches!(
            s,
            PublicationStatus::Failed { .. } | PublicationStatus::Ready { .. }
        )
    })
    .await;
    assert_eq!(
        f.db.lock()
            .convoy_stamp()
            .expect("read fixture publication stamp"),
        before,
        "candidate changed the tracked revision or committed receipt"
    );
    assert!(matches!(end, PublicationStatus::Failed { .. }));
    assert!(matches!(
        publisher.admit(None, None).await,
        Err(DeliveryError::PublishFailed)
    ));
}

#[tokio::test]
async fn v2_revision_change_during_copy_retries_without_stale_receipt() {
    const PLUGIN: &str = "BepInEx/plugins/stamp/r1-stamp.dll";
    let f = Fixture::new();
    f.track(PLUGIN, &vec![b'a'; 128 * 1024]);
    let publisher = f.publisher();
    let old = ready(&publisher).await;
    let before =
        f.db.lock()
            .convoy_stamp()
            .expect("read fixture publication stamp");
    f.write(PLUGIN, &vec![b'b'; 128 * 1024]);
    let (entered, resume) = arm(PLUGIN);
    publisher.refresh(PublishMode::Rehash);
    entered
        .recv_timeout(Duration::from_secs(10))
        .expect("worker must reach barrier");
    let writer = Database::open(&f.dirs.db_path()).expect("open fixture file or database");
    writer
        .conn()
        .execute(
            "UPDATE installed_mods SET name = 'New revision' WHERE id = ?1",
            [f.id],
        )
        .expect("fixture operation must succeed");
    let after = writer
        .convoy_stamp()
        .expect("read fixture publication stamp");
    assert!(after.0 > before.0);
    assert_eq!(after.1, before.1);
    assert!(matches!(
        publisher.admit(None, None).await,
        Err(DeliveryError::Preparing)
    ));
    let (retry, finish) = arm(PLUGIN);
    resume.send(()).expect("worker must accept barrier release");
    retry
        .recv_timeout(Duration::from_secs(10))
        .expect("worker must reach barrier");
    assert_eq!(
        writer
            .convoy_stamp()
            .expect("read fixture publication stamp"),
        after,
        "superseded attempt committed before retry"
    );
    finish.send(()).expect("worker must accept barrier release");
    assert_ne!(ready(&publisher).await, old);
    assert_ne!(
        writer
            .convoy_stamp()
            .expect("read fixture publication stamp")
            .1,
        before.1
    );
}

#[tokio::test]
async fn v2_cold_candidate_is_reclaimed_when_promotion_is_disabled() {
    let f = Fixture::new();
    f.track("BepInEx/plugins/cold/r1-cold.dll", b"new candidate");
    let (entered, resume) = arm(f.dirs.root.clone());
    let publisher = f.publisher();
    entered
        .recv_timeout(Duration::from_secs(10))
        .expect("worker must reach barrier");
    let store = f.dirs.convoy_store_dir();
    assert_eq!(
        store.read_dir().expect("list fixture store").count(),
        1,
        "candidate did not finish"
    );
    f.config
        .write()
        .convoy
        .as_mut()
        .expect("fixture has Convoy configuration")
        .enabled = false;
    resume.send(()).expect("worker must accept barrier release");
    wait(&publisher, |s| *s == PublicationStatus::Disabled).await;
    assert_eq!(
        f.db.lock()
            .convoy_stamp()
            .expect("read fixture publication stamp")
            .1,
        None
    );
    assert_eq!(
        store.read_dir().expect("list fixture store").count(),
        0,
        "disabled publisher leaked candidate-owned bytes"
    );
}

#[tokio::test]
async fn v2_retry_preserves_rehash_when_a_pin_drops_during_the_attempt() {
    use sha2::{Digest, Sha256};
    const PLUGIN: &str = "BepInEx/plugins/pin/r1-pin.dll";
    let f = Fixture::new();
    f.config
        .write()
        .convoy
        .as_mut()
        .expect("fixture has Convoy configuration")
        .max_store_bytes = 7;
    f.track(PLUGIN, b"abc");
    let publisher = f.publisher();
    let old = ready(&publisher).await;
    let digest = format!("{:x}", Sha256::digest(b"abc"));
    let lease = publisher
        .admit(Some(&old), Some(&digest))
        .await
        .expect("fixture operation must succeed");
    f.write(PLUGIN, b"def");
    publisher.refresh(PublishMode::Rehash);
    let current = ready(&publisher).await;
    assert_ne!(old, current);
    let (entered, resume) = arm(f.dirs.root.clone());
    f.write(PLUGIN, b"ghi");
    publisher.refresh(PublishMode::Rehash);
    entered
        .recv_timeout(Duration::from_secs(10))
        .expect("worker must reach barrier");
    drop(lease);
    resume.send(()).expect("worker must accept barrier release");
    assert!(matches!(
        wait(
            &publisher,
            |s| matches!(s, PublicationStatus::Ready { generation, .. } if generation != &current)
        )
        .await,
        PublicationStatus::Ready { .. }
    ));
    let recorded: String =
        f.db.lock()
            .conn()
            .query_row("SELECT file_hash FROM installed_files", [], |r| r.get(0))
            .expect("fixture operation must succeed");
    assert_eq!(recorded, hash(b"ghi"));
}

async fn corrupt_receipt(f: &Fixture, corruption: &str) {
    let publisher = f.publisher();
    ready(&publisher).await;
    drop(publisher);
    let db = f.db.lock();
    let stored = db
        .convoy_receipt()
        .expect("read stored fixture receipt")
        .expect("read stored fixture receipt");
    let (id, data) = match corruption {
        "identity" => (stored.id, b"invalid".to_vec()),
        "parse" => (receipt::digest(b"invalid"), b"invalid".to_vec()),
        "seal" => {
            let mut receipt: serde_json::Value =
                serde_json::from_slice(&stored.data).expect("decode or encode fixture receipt");
            receipt["manifest"]["generation"] = serde_json::Value::String("invalid".into());
            let data = serde_json::to_vec(&receipt).expect("decode or encode fixture receipt");
            (receipt::digest(&data), data)
        }
        _ => unreachable!(),
    };
    db.conn()
        .execute(
            "UPDATE convoy_publication SET receipt = ?1, receipt_data = ?2 WHERE id = 1",
            rusqlite::params![id, data],
        )
        .expect("fixture operation must succeed");
}

#[tokio::test]
async fn v2_cold_rehash_replaces_invalid_receipt_only_after_verified_cas() {
    use std::io::Read;
    const PLUGIN: &str = "BepInEx/plugins/cold/r2-recovery.dll";
    for corruption in ["identity", "parse", "seal"] {
        let f = Fixture::new();
        f.track(PLUGIN, b"old bytes");
        corrupt_receipt(&f, corruption).await;
        f.write(PLUGIN, b"verified replacement");
        let before =
            f.db.lock()
                .convoy_stamp()
                .expect("read fixture publication stamp");
        let publisher = f.publisher();
        wait(&publisher, |s| {
            matches!(s, PublicationStatus::Failed { .. })
        })
        .await;
        assert!(matches!(
            publisher.admit(None, None).await,
            Err(DeliveryError::PublishFailed)
        ));
        let (entered, resume) = arm(f.dirs.root.clone());
        publisher.refresh(PublishMode::Rehash);
        entered
            .recv_timeout(Duration::from_secs(10))
            .expect("worker must reach barrier");
        let store = f.dirs.convoy_store_dir();
        let old = store.join(receipt::digest(b"old bytes"));
        let new_digest = receipt::digest(b"verified replacement");
        assert_eq!(
            std::fs::read(&old).expect("read fixture bytes"),
            b"old bytes",
            "{corruption}"
        );
        assert_eq!(
            std::fs::read(store.join(&new_digest))
                .expect("verified candidate must stage before CAS"),
            b"verified replacement",
            "{corruption}"
        );
        assert_eq!(
            f.db.lock()
                .convoy_stamp()
                .expect("read fixture publication stamp"),
            before
        );
        assert!(matches!(
            publisher.admit(None, None).await,
            Err(DeliveryError::Preparing)
        ));
        resume.send(()).expect("worker must accept barrier release");
        let generation = ready(&publisher).await;
        {
            let db = f.db.lock();
            let updated = db.convoy_stamp().expect("read fixture publication stamp");
            let stored = db
                .convoy_receipt()
                .expect("read stored fixture receipt")
                .expect("read stored fixture receipt");
            assert!(updated.0 > before.0);
            assert_eq!(
                updated,
                (stored.revision, Some(receipt::digest(&stored.data)))
            );
            let checksum: String = db
                .conn()
                .query_row("SELECT file_hash FROM installed_files", [], |r| r.get(0))
                .expect("fixture operation must succeed");
            assert_eq!(checksum, hash(b"verified replacement"));
        }
        assert!(!old.exists());
        let Admission::Object { mut file, .. } = publisher
            .admit(Some(&generation), Some(&new_digest))
            .await
            .expect("fixture operation must succeed")
        else {
            panic!("expected verified object")
        };
        let mut actual = Vec::new();
        file.read_to_end(&mut actual)
            .expect("read verified object body");
        assert_eq!(actual, b"verified replacement");
    }
}

#[tokio::test]
async fn v2_cold_rehash_keeps_invalid_receipt_and_old_bytes_at_hard_cap() {
    const PLUGIN: &str = "BepInEx/plugins/cold/r2-hard-cap.dll";
    let f = Fixture::new();
    f.track(PLUGIN, b"old bytes");
    corrupt_receipt(&f, "parse").await;
    let before =
        f.db.lock()
            .convoy_stamp()
            .expect("read fixture publication stamp");
    f.config
        .write()
        .convoy
        .as_mut()
        .expect("fixture has Convoy configuration")
        .max_store_bytes = 12;
    f.write(PLUGIN, b"new bytes");
    let publisher = f.publisher();
    wait(&publisher, |s| {
        matches!(s, PublicationStatus::Failed { .. })
    })
    .await;
    publisher.refresh(PublishMode::Rehash);
    assert_eq!(
        wait(&publisher, |s| matches!(
            s,
            PublicationStatus::Failed { .. } | PublicationStatus::Ready { .. }
        ))
        .await,
        PublicationStatus::Failed {
            reason: "store_capacity"
        }
    );
    assert_eq!(
        f.db.lock()
            .convoy_stamp()
            .expect("read fixture publication stamp"),
        before
    );
    let store = f.dirs.convoy_store_dir();
    assert_eq!(store.read_dir().expect("list fixture store").count(), 1);
    assert_eq!(
        std::fs::read(store.join(receipt::digest(b"old bytes"))).expect("read fixture bytes"),
        b"old bytes"
    );
}

#[tokio::test]
async fn v2_aborted_warm_candidate_keeps_the_committed_and_leased_digest() {
    use sha2::{Digest, Sha256};
    const PLUGIN: &str = "BepInEx/plugins/warm/r1-warm.dll";
    let f = Fixture::new();
    f.track(PLUGIN, b"original bytes");
    let publisher = f.publisher();
    let generation = ready(&publisher).await;
    let digest = format!("{:x}", Sha256::digest(b"original bytes"));
    let lease = publisher
        .admit(Some(&generation), Some(&digest))
        .await
        .expect("fixture operation must succeed");
    let before =
        f.db.lock()
            .convoy_stamp()
            .expect("read fixture publication stamp");
    let (entered, resume) = arm(f.dirs.root.clone());
    f.write(PLUGIN, b"discarded bytes");
    publisher.refresh(PublishMode::Rehash);
    entered
        .recv_timeout(Duration::from_secs(10))
        .expect("worker must reach barrier");
    let store = f.dirs.convoy_store_dir();
    assert_eq!(store.read_dir().expect("list fixture store").count(), 2);
    f.config
        .write()
        .convoy
        .as_mut()
        .expect("fixture has Convoy configuration")
        .enabled = false;
    resume.send(()).expect("worker must accept barrier release");
    wait(&publisher, |s| *s == PublicationStatus::Disabled).await;
    assert_eq!(
        f.db.lock()
            .convoy_stamp()
            .expect("read fixture publication stamp"),
        before
    );
    assert_eq!(store.read_dir().expect("list fixture store").count(), 1);
    assert_eq!(
        std::fs::read(store.join(digest)).expect("read fixture bytes"),
        b"original bytes"
    );
    drop(lease);
}
