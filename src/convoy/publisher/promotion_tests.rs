use super::super::test_sync::{arm, ready, Fixture};
use super::*;
use actix_web::{dev::Service, web, App, HttpServer};
use std::net::TcpListener;
use tokio::sync::Notify;

const MANIFEST: &str = "/quma/convoy/v2/catalog";
const PING: &str = "/launcher/v2/ping";

fn app_state(
    f: &Fixture,
    p: web::Data<Publisher>,
    upstream: u16,
) -> web::Data<crate::web::state::AppState> {
    use crate::{
        config_mgmt::ConfigManager,
        forge::client::ForgeClient,
        logging,
        spt::{detect::SptInfo, game_data::GameData},
        web::{
            integrity_cache::IntegrityCache, mod_zip_cache::ModZipCache,
            proxy_metrics::ProxyMetrics, state::AppState, tasks::TaskTracker,
            update_cache::UpdateCache,
        },
    };
    use std::sync::atomic::AtomicUsize;
    let (events, _) = tokio::sync::broadcast::channel(16);
    {
        let mut config = f.config.write();
        config.proxy_enabled = true;
        config.proxy_auth = false;
        config.server_port = Some(upstream);
    }
    web::Data::new(AppState {
        db: f.db.clone(),
        forge: ForgeClient::with_base_url("http://127.0.0.1:1".into())
            .expect("create unused fixture Forge client"),
        config: f.config.clone(),
        config_path: f.dirs.config_path(),
        config_lock: Default::default(),
        dirs: Arc::new(f.dirs.clone()),
        spt_info: SptInfo {
            root: f.dirs.spt_server.clone(),
            spt_version: "fixture".into(),
            tarkov_version: "fixture".into(),
        },
        tasks: TaskTracker::new(events.clone()),
        update_cache: UpdateCache::new(300),
        integrity_cache: IntegrityCache::new(300),
        events,
        log_broadcast: Arc::new(logging::LogBroadcast::new(16)),
        reload_handles: Arc::new(logging::init_reload_handles_only()),
        container_mgr: None,
        converging: Default::default(),
        fika_installed: false,
        svm: Arc::new(parking_lot::RwLock::new(None)),
        config_mgmt: ConfigManager::new(&f.dirs),
        server_transition: Default::default(),
        game_data: Arc::new(GameData::load_empty()),
        proxy_metrics: ProxyMetrics::new(),
        proxy_client: reqwest::Client::builder()
            .no_proxy()
            .danger_accept_invalid_certs(true)
            .build()
            .expect("create disposable TLS client"),
        mod_zip_cache: ModZipCache::new(f.dirs.clone(), f.db.clone(), f.config.clone()),
        log_level_counts: Default::default(),
        fika_client: None,
        fika_config_lock: Default::default(),
        convoy_publisher: p,
        fika_items: Default::default(),
        headless_service: None,
        sse_connections: AtomicUsize::new(0),
    })
}

async fn error_response(response: reqwest::Response, code: &str, retry: bool) -> Result<()> {
    assert_eq!(response.status(), 503);
    if retry {
        assert_eq!(response.headers()["retry-after"], "1");
    }
    assert_eq!(response.json::<serde_json::Value>().await?["code"], code);
    Ok(())
}

#[actix_web::test]
async fn wave_promotion_keeps_single_worker_live_and_rejects_old_ticket() -> Result<()> {
    let f = Fixture::new();
    const PLUGIN: &str = "BepInEx/plugins/wave/promote.dll";
    const OLD: &[u8] = b"old bytes";
    const NEW: &[u8] = b"new bytes";
    f.track(PLUGIN, OLD);
    let p = web::Data::new(f.publisher());
    let old = ready(&p).await;
    let old_digest = receipt::digest(OLD);
    let new_digest = receipt::digest(NEW);
    let upstream = TcpListener::bind("127.0.0.1:0")?;
    let upstream_port = upstream.local_addr()?.port();
    let cert = rcgen::generate_simple_self_signed(vec!["localhost".into()])?;
    let tls = rustls::ServerConfig::builder()
        .with_no_client_auth()
        .with_single_cert(
            vec![cert.cert.der().clone()],
            rustls::pki_types::PrivatePkcs8KeyDer::from(cert.signing_key.serialize_der()).into(),
        )?;
    let upstream_server =
        HttpServer::new(|| App::new().route(PING, web::get().to(|| async { "real upstream" })))
            .workers(1)
            .disable_signals()
            .listen_rustls_0_23(upstream, tls)?
            .run();
    let upstream_handle = upstream_server.handle();
    let upstream_task = actix_web::rt::spawn(upstream_server);
    let socket = TcpListener::bind("127.0.0.1:0")?;
    let origin = format!("http://{}", socket.local_addr()?);
    let data = app_state(&f, p.clone(), upstream_port);
    let pinned = Arc::new(Notify::new());
    let finish_body = Arc::new(Notify::new());
    let server = {
        let (pinned, finish_body) = (pinned.clone(), finish_body.clone());
        HttpServer::new(move || {
            let (pinned, finish_body) = (pinned.clone(), finish_body.clone());
            App::new()
                .app_data(data.clone())
                .app_data(data.convoy_publisher.clone())
                .wrap_fn(move |req, svc| {
                    let hold = req.headers().contains_key("x-hold-fixture-body");
                    let (pinned, finish_body) = (pinned.clone(), finish_body.clone());
                    let call = svc.call(req);
                    async move {
                        let response = call.await?;
                        if hold && response.status().is_success() {
                            pinned.notify_one();
                            finish_body.notified().await;
                        }
                        Ok(response)
                    }
                })
                .service(
                    web::scope("/quma").configure(crate::web::handlers::convoy_v2::configure_v2),
                )
                .route(PING, web::get().to(crate::web::proxy::proxy_handler))
        })
        .workers(1)
        .disable_signals()
        .listen(socket)?
        .run()
    };
    let handle = server.handle();
    let task = actix_web::rt::spawn(server);
    let http = reqwest::Client::builder()
        .no_proxy()
        .timeout(Duration::from_secs(5))
        .build()?;
    let old_url = format!("{origin}/quma/convoy/v2/generations/{old}/objects/{old_digest}");
    let manifest = format!("{origin}{MANIFEST}");

    // Stop the real stamp receiver with its first admitted query, then fill
    // every remaining slot deterministically (no scheduler/count stress race).
    let (stamp_entered, stamp_resume) = arm(f.dirs.root.join("stamp"));
    let (probe, stamp_result) = oneshot::channel();
    p.stamps.try_send(probe)?;
    stamp_entered.recv_timeout(Duration::from_secs(5))?;
    let mut queued = Vec::new();
    for _ in 0..STAMP_QUEUE {
        let (probe, pending) = oneshot::channel();
        p.stamps.try_send(probe)?;
        queued.push(pending);
    }
    error_response(http.get(&manifest).send().await?, "busy", true).await?;
    stamp_resume.send(())?;
    stamp_result.await?.expect("real database stamp");
    for pending in queued {
        pending.await?.expect("queued real database stamp");
    }

    let body = actix_web::rt::spawn({
        let request = http.get(&old_url).header("x-hold-fixture-body", "1");
        async move { request.send().await?.error_for_status()?.bytes().await }
    });
    tokio::time::timeout(Duration::from_secs(5), pinned.notified()).await?;
    assert!(p.state.lock().pins.contains_key(&old_digest));
    let before = f.db.lock().convoy_stamp()?;
    let (entered, resume) = arm(f.dirs.root.join("promotion"));
    f.write(PLUGIN, NEW);
    p.refresh(PublishMode::Rehash);
    entered.recv_timeout(Duration::from_secs(5))?;
    assert!(
        p.state.try_lock().is_some() && p.config.try_write().is_some(),
        "SQLite promotion holds request/lease or config lock"
    );
    assert_eq!(f.db.lock().convoy_stamp()?, before);
    assert_eq!(p.status(), PublicationStatus::Preparing);
    error_response(http.get(&manifest).send().await?, "preparing", true).await?;
    error_response(http.get(&old_url).send().await?, "preparing", true).await?;
    assert_eq!(
        http.get(format!("{origin}{PING}"))
            .send()
            .await?
            .error_for_status()?
            .text()
            .await?,
        "real upstream"
    );

    let policy = f.config.read().convoy.clone();
    f.config
        .write()
        .convoy
        .as_mut()
        .expect("fixture policy")
        .enabled = false;
    p.refresh(PublishMode::Managed);
    error_response(http.get(&manifest).send().await?, "disabled", false).await?;
    f.config.write().convoy = policy;
    p.refresh(PublishMode::Managed);
    finish_body.notify_one();
    assert_eq!(body.await??.as_ref(), OLD);
    tokio::time::timeout(Duration::from_secs(2), async {
        while p.state.lock().pins.contains_key(&old_digest) {
            tokio::time::sleep(Duration::from_millis(1)).await;
        }
    })
    .await?;

    // Stop the replacement job before CAS; the transaction paused above
    // must have committed, but its pre-disable ticket cannot be admitted.
    let (retrying, continue_retry) = arm(f.dirs.root.clone());
    resume.send(())?;
    retrying.recv_timeout(Duration::from_secs(5))?;
    let durable = f.db.lock().convoy_stamp()?;
    assert_ne!(durable, before);
    let store = f.dirs.convoy_store_dir();
    assert_eq!(std::fs::read(store.join(&new_digest))?, NEW);
    assert!(!store.join(&old_digest).exists());
    assert_eq!(p.status(), PublicationStatus::Preparing);
    error_response(http.get(&manifest).send().await?, "preparing", true).await?;
    let candidate_generation = {
        let state = p.state.lock();
        let active = state.active.as_ref().expect("durable candidate retained");
        assert_ne!(state.ticket, active.ticket);
        active.snapshot.manifest.generation.clone()
    };
    continue_retry.send(())?;
    assert_eq!(ready(&p).await, candidate_generation);
    let new_url =
        format!("{origin}/quma/convoy/v2/generations/{candidate_generation}/objects/{new_digest}");
    assert_eq!(
        http.get(new_url)
            .send()
            .await?
            .error_for_status()?
            .bytes()
            .await?
            .as_ref(),
        NEW
    );
    assert_eq!(store.read_dir()?.count(), 1);
    handle.stop(true).await;
    task.await??;
    upstream_handle.stop(true).await;
    upstream_task.await??;
    Ok(())
}

#[tokio::test]
async fn wave_failed_promotion_rolls_back_receipt_rows_and_new_objects() -> Result<()> {
    const PLUGIN: &str = "BepInEx/plugins/wave/rollback.dll";
    const OLD: &[u8] = b"old receipt";
    const NEW: &[u8] = b"aborted candidate";
    let f = Fixture::new();
    f.track(PLUGIN, OLD);
    let p = f.publisher();
    let old = ready(&p).await;
    let before = f.db.lock().convoy_stamp()?;
    f.db.lock().conn().execute_batch("CREATE TRIGGER fail_disposable_promotion BEFORE UPDATE OF receipt ON convoy_publication BEGIN SELECT RAISE(ABORT, 'injected receipt write failure'); END;")?;
    f.write(PLUGIN, NEW);
    p.refresh(PublishMode::Rehash);
    let mut status = p.subscribe();
    tokio::time::timeout(
        Duration::from_secs(5),
        status.wait_for(|s| {
            *s == PublicationStatus::Failed {
                reason: "commit_failed",
            }
        }),
    )
    .await??;
    assert_eq!(f.db.lock().convoy_stamp()?, before);
    let checksum: String =
        f.db.lock()
            .conn()
            .query_row("SELECT file_hash FROM installed_files", [], |r| r.get(0))?;
    assert_eq!(
        checksum,
        format!("{:016x}", xxhash_rust::xxh3::xxh3_64(OLD))
    );
    let store = f.dirs.convoy_store_dir();
    let aborted = store.join(receipt::digest(NEW));
    tokio::time::timeout(Duration::from_secs(2), async {
        while aborted.exists() {
            tokio::time::sleep(Duration::from_millis(1)).await;
        }
    })
    .await?;
    assert_eq!(std::fs::read(store.join(receipt::digest(OLD)))?, OLD);
    f.db.lock()
        .conn()
        .execute_batch("DROP TRIGGER fail_disposable_promotion;")?;
    p.refresh(PublishMode::Rehash);
    assert_ne!(ready(&p).await, old);
    assert_eq!(std::fs::read(aborted)?, NEW);
    assert_eq!(store.read_dir()?.count(), 1);
    Ok(())
}
