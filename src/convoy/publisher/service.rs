use super::*;
use parking_lot::Mutex;
use std::{sync::mpsc, time::Duration};
use tokio::sync::{oneshot, watch, OwnedSemaphorePermit, Semaphore};

const POLL: Duration = Duration::from_millis(250);
const STAMP_QUEUE: usize = 32;
// File handles preserve inode identity but must not exhaust the process limit.
// Objects beyond this bound are retained and still count against storage.
const MAX_RETIRED_HANDLES: usize = 64;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PublicationStatus {
    Disabled,
    Preparing,
    Ready { generation: String, bytes: u64 },
    Failed { reason: &'static str },
}

struct Active {
    admissible: bool,
    revision: i64,
    receipt: String,
    policy: ConvoyConfig,
    ticket: u64,
    snapshot: Arc<PublishedSnapshot>,
    store: ObjectStore,
}

#[derive(Default)]
struct State {
    ticket: u64,
    pending: Option<PublishMode>,
    active: Option<Active>,
    pins: BTreeMap<String, (usize, u64)>,
    retired: BTreeMap<String, (ObjectStore, File, ObjectRecord)>,
    pin_wait: Option<PinWait>,
    collected_dirty: bool,
    promoting: bool,
    stopped: bool,
}

struct PinWait {
    mode: PublishMode,
    deficit: u64,
    retired_pins: BTreeMap<String, u64>,
}

impl PinWait {
    fn can_retry(&self, state: &State) -> bool {
        self.retired_pins
            .iter()
            .filter(|(digest, _)| !state.pins.contains_key(*digest))
            .map(|(_, bytes)| bytes)
            .sum::<u64>()
            >= self.deficit
    }
}

type Stamp = (i64, Option<String>);
type StampReply = oneshot::Sender<Result<Stamp, DeliveryError>>;

pub struct Publisher {
    config: Arc<RwLock<Config>>,
    state: Arc<Mutex<State>>,
    status: Arc<watch::Sender<PublicationStatus>>,
    wake: mpsc::SyncSender<()>,
    stamps: mpsc::SyncSender<StampReply>,
    bodies: Arc<Semaphore>,
}

pub struct ObjectLease {
    _permit: OwnedSemaphorePermit,
    digest: String,
    state: Arc<Mutex<State>>,
}

impl Drop for ObjectLease {
    fn drop(&mut self) {
        let mut state = self.state.lock();
        if let Some((count, _)) = state.pins.get_mut(&self.digest) {
            *count -= 1;
            if *count == 0 {
                state.pins.remove(&self.digest);
                state.collected_dirty = true;
            }
        }
    }
}

impl State {
    fn reachable(&self) -> std::collections::BTreeSet<String> {
        self.pins
            .keys()
            .chain(
                self.active
                    .iter()
                    .flat_map(|active| active.snapshot.objects.keys()),
            )
            .cloned()
            .collect()
    }
}

impl Drop for Publisher {
    fn drop(&mut self) {
        self.state.lock().stopped = true;
        let _ = self.wake.try_send(());
    }
}

impl Publisher {
    pub fn start(
        dirs: QumaDirs,
        config: Arc<RwLock<Config>>,
        db: Arc<Mutex<Database>>,
    ) -> Result<Self> {
        let stamp_db = Database::open_read_only(&dirs.db_path())?;
        stamp_db.conn().busy_timeout(Duration::from_millis(200))?;
        let capture_db = Database::open_read_only(&dirs.db_path())?;
        let writer = Database::open(&dirs.db_path())?;
        writer.conn().busy_timeout(Duration::ZERO)?;
        let (stamps, queries) = mpsc::sync_channel::<StampReply>(STAMP_QUEUE);
        #[cfg(test)]
        let stamp_gate = dirs.root.join("stamp");
        std::thread::Builder::new()
            .name("convoy-stamp".into())
            .spawn(move || {
                while let Ok(reply) = queries.recv() {
                    #[cfg(test)]
                    super::test_sync::pause(&stamp_gate);
                    if !reply.is_closed() {
                        let _ = reply.send(
                            stamp_db
                                .convoy_stamp()
                                .map_err(|_| DeliveryError::PublishFailed),
                        );
                    }
                }
            })?;
        let (wake, jobs) = mpsc::sync_channel(1);
        let state = Arc::new(Mutex::new(State::default()));
        let status = Arc::new(watch::Sender::new(if policy(&config).is_some() {
            PublicationStatus::Preparing
        } else {
            PublicationStatus::Disabled
        }));
        let worker = Worker {
            dirs,
            config: config.clone(),
            state: state.clone(),
            status: status.clone(),
            capture_db,
            writer,
            _db: db,
        };
        std::thread::Builder::new()
            .name("convoy-publish".into())
            .spawn(move || worker.run(jobs))?;
        Ok(Self {
            config,
            state,
            status,
            wake,
            stamps,
            bodies: Arc::new(Semaphore::new(16)),
        })
    }

    pub fn refresh(&self, mode: PublishMode) {
        let enabled = policy(&self.config).is_some();
        let mut state = self.state.lock();
        state.ticket = state.ticket.wrapping_add(1);
        if !matches!(state.pending, Some(PublishMode::Rehash)) {
            state.pending = Some(mode);
        }
        self.status.send_replace(if enabled {
            PublicationStatus::Preparing
        } else {
            PublicationStatus::Disabled
        });
        let _ = self.wake.try_send(());
    }

    pub fn status(&self) -> PublicationStatus {
        let config = self.config.read();
        let Some(policy) = config.convoy.as_ref().filter(|p| p.enabled) else {
            return PublicationStatus::Disabled;
        };
        let state = self.state.lock();
        let current = self.status.borrow().clone();
        if matches!(current, PublicationStatus::Ready { .. })
            && state.active.as_ref().is_some_and(|a| &a.policy != policy)
        {
            return PublicationStatus::Preparing;
        }
        current
    }

    pub fn subscribe(&self) -> watch::Receiver<PublicationStatus> {
        self.status.subscribe()
    }

    pub async fn admit(
        &self,
        generation: Option<&str>,
        digest: Option<&str>,
    ) -> Result<Admission, DeliveryError> {
        if policy(&self.config).is_none() {
            self.refresh(PublishMode::Managed);
            return Err(DeliveryError::Disabled);
        }
        if self.state.lock().promoting {
            return Err(DeliveryError::Preparing);
        }
        let (tx, rx) = oneshot::channel();
        self.stamps.try_send(tx).map_err(|err| match err {
            mpsc::TrySendError::Full(_) => DeliveryError::Busy,
            mpsc::TrySendError::Disconnected(_) => DeliveryError::PublishFailed,
        })?;
        let (revision, receipt) = rx.await.map_err(|_| DeliveryError::PublishFailed)??;
        let (store, digest, record, lease) = {
            let config = self.config.read();
            let Some(policy) = config.convoy.as_ref().filter(|p| p.enabled) else {
                return Err(DeliveryError::Disabled);
            };
            let mut state = self.state.lock();
            let Some(active) = state.active.as_ref().filter(|active| {
                !state.promoting
                    && active.admissible
                    && active.revision == revision
                    && receipt.as_ref() == Some(&active.receipt)
                    && &active.policy == policy
                    && active.ticket == state.ticket
            }) else {
                let _ = self.wake.try_send(());
                return Err(match &*self.status.borrow() {
                    PublicationStatus::Failed { .. } => DeliveryError::PublishFailed,
                    _ => DeliveryError::Preparing,
                });
            };
            let snapshot = &active.snapshot;
            let (generation, digest) = match (generation, digest) {
                (None, None) => {
                    return Ok(Admission::Manifest {
                        generation: snapshot.manifest.generation.clone(),
                        json: snapshot.json.clone(),
                    })
                }
                (Some(generation), Some(digest)) => (generation, digest),
                _ => return Err(DeliveryError::NotPublished),
            };
            if snapshot.manifest.generation != generation {
                return Err(DeliveryError::StaleGeneration);
            }
            let record = snapshot
                .objects
                .get(digest)
                .ok_or(DeliveryError::NotPublished)?
                .clone();
            let permit = self
                .bodies
                .clone()
                .try_acquire_owned()
                .map_err(|_| DeliveryError::Busy)?;
            let store = active.store.clone();
            let digest = digest.to_owned();
            state
                .pins
                .entry(digest.clone())
                .or_insert((0, record.length))
                .0 += 1;
            let lease = ObjectLease {
                _permit: permit,
                digest: digest.clone(),
                state: self.state.clone(),
            };
            (store, digest, record, lease)
        };
        let length = record.length;
        let (file, lease) = tokio::task::spawn_blocking(move || {
            store.lease(&digest, &record).map(|file| (file, lease))
        })
        .await
        .map_err(|_| DeliveryError::PublishFailed)?
        .map_err(|err| {
            tracing::warn!(%err, "failed to open verified Convoy object");
            DeliveryError::PublishFailed
        })?;
        Ok(Admission::Object {
            file,
            length,
            lease,
        })
    }
}

fn policy(config: &RwLock<Config>) -> Option<ConvoyConfig> {
    config.read().convoy.clone().filter(|p| p.enabled)
}

struct Worker {
    dirs: QumaDirs,
    config: Arc<RwLock<Config>>,
    state: Arc<Mutex<State>>,
    status: Arc<watch::Sender<PublicationStatus>>,
    capture_db: Database,
    writer: Database,
    _db: Arc<Mutex<Database>>,
}

impl Worker {
    fn run(self, jobs: mpsc::Receiver<()>) {
        let mut attempted = None;
        while !self.state.lock().stopped {
            self.collect_retired();
            self.reconcile(&mut attempted);
            if let Err(mpsc::RecvTimeoutError::Disconnected) = jobs.recv_timeout(POLL) {
                break;
            }
        }
    }

    fn collect_retired(&self) {
        let job = {
            let mut state = self.state.lock();
            if !std::mem::take(&mut state.collected_dirty) {
                return;
            }
            let keep = state.reachable();
            let keys: Vec<_> = state
                .retired
                .keys()
                .filter(|digest| !keep.contains(*digest))
                .cloned()
                .collect();
            keys.into_iter()
                .filter_map(|digest| state.retired.remove(&digest).map(|object| (digest, object)))
                .collect::<Vec<_>>()
        };
        for (digest, (store, held, record)) in job {
            if let Err(err) = store.retire(&digest, &record, &held) {
                tracing::warn!(%err, %digest, "cannot safely collect retired Convoy object; retaining it");
            }
        }
    }

    fn reconcile(&self, attempted: &mut Option<(Stamp, ConvoyConfig, u64)>) {
        let Some(policy) = policy(&self.config) else {
            let mut state = self.state.lock();
            if state
                .active
                .as_ref()
                .is_some_and(|a| a.ticket == state.ticket)
            {
                state.ticket = state.ticket.wrapping_add(1);
            }
            self.status.send_replace(PublicationStatus::Disabled);
            *attempted = None;
            return;
        };
        let Ok(stamp) = self.capture_db.convoy_stamp() else {
            self.status.send_replace(PublicationStatus::Failed {
                reason: "stamp_unavailable",
            });
            return;
        };
        let (ticket, mode, previous) = {
            let mut state = self.state.lock();
            let ticket = state.ticket;
            if attempted
                .as_ref()
                .is_some_and(|key| key == &(stamp.clone(), policy.clone(), ticket))
                && !state
                    .pin_wait
                    .as_ref()
                    .is_some_and(|wait| wait.can_retry(&state))
            {
                return;
            }
            self.status.send_replace(PublicationStatus::Preparing);
            let waiting = state.pin_wait.take().map(|w| w.mode);
            let requested = state.pending.take();
            let mode = if matches!(waiting, Some(PublishMode::Rehash))
                || matches!(requested, Some(PublishMode::Rehash))
            {
                PublishMode::Rehash
            } else {
                PublishMode::Managed
            };
            (
                ticket,
                mode,
                state.active.as_ref().map(|active| active.snapshot.clone()),
            )
        };
        let mut observed = stamp;
        let mut retired_pins = BTreeMap::new();
        let mut owned = None;
        let result = (|| -> Result<_> {
            let recovering = previous.is_none();
            let mut input =
                CatalogInput::capture_metadata(&self.capture_db, &self.dirs, &policy, recovering)?;
            observed = input.stamp.clone();
            let store = ObjectStore::new(&self.dirs.convoy_store_dir(), policy.max_store_bytes)?;
            let (runtime, spt_version) = installation(&input)?;
            let mut updates = Rebaseline::new();
            let mut witnesses = Witnesses::default();
            let legacy_receipt = input
                .committed_receipt
                .as_ref()
                .is_some_and(receipt::Receipt::is_legacy);
            let restored = input
                .committed_receipt
                .as_ref()
                .filter(|_| matches!(mode, PublishMode::Managed) && !legacy_receipt)
                .map(|stored| {
                    receipt::Receipt::restore(
                        stored,
                        &input,
                        &policy,
                        runtime,
                        &spt_version,
                        &store,
                    )
                })
                .transpose()?;
            let mut snapshot = if let Some((true, snapshot)) = restored {
                snapshot
            } else {
                let previous = previous
                    .as_deref()
                    .or(restored.as_ref().map(|(_, snapshot)| snapshot));
                {
                    let state = self.state.lock();
                    for (digest, (_, length)) in &state.pins {
                        if !state
                            .active
                            .as_ref()
                            .is_some_and(|active| active.snapshot.objects.contains_key(digest))
                        {
                            retired_pins.insert(digest.clone(), *length);
                        }
                    }
                }
                match input
                    .committed_receipt
                    .as_ref()
                    .filter(|_| !legacy_receipt)
                    .map(receipt::Receipt::committed_digests)
                    .transpose()
                {
                    Ok(_) => {}
                    Err(err) if matches!(mode, PublishMode::Rehash) => {
                        // An unreadable receipt is not a deletion allowlist.
                        // Stage against all retained bytes and let successful
                        // compare-and-publish drive ordinary retirement.
                        tracing::warn!(%err, "deferring Convoy collection until receipt replacement");
                    }
                    Err(err) => return Err(err),
                }
                let (snapshot, ownership) = build_candidate(
                    &input,
                    &store,
                    previous,
                    mode,
                    &mut updates,
                    (runtime, &spt_version),
                    &mut witnesses,
                )?;
                owned = Some(ownership);
                snapshot
            };
            ensure!(
                installation(&input)? == (runtime, spt_version),
                "installation changed during publication"
            );
            input.apply_rebaseline(&updates);
            for m in &input.mods {
                if let Some(p) = snapshot.sources.get_mut(&m.id) {
                    p.key = input.package_key(m, runtime, &snapshot.manifest.spt_version)?;
                }
            }
            let bytes = receipt::Receipt::encode(&input, &policy, runtime, &snapshot)?;
            let id = receipt::digest(&bytes);
            witnesses.validate(&self.dirs)?;
            Ok((store, snapshot, id, bytes, updates))
        })();
        #[cfg(test)]
        super::test_sync::pause(&self.dirs.root);
        let stamp = self.capture_db.convoy_stamp();
        let still_current = |state: &State, config: &Config| {
            !state.stopped && state.ticket == ticket && config.convoy.as_ref() == Some(&policy)
        };
        {
            let config = self.config.read();
            let mut state = self.state.lock();
            if !still_current(&state, &config) || stamp.ok().as_ref() != Some(&observed) {
                if matches!(mode, PublishMode::Rehash) {
                    state.pending = Some(PublishMode::Rehash);
                }
                return;
            }
            if result.is_ok() {
                state.promoting = true;
                self.status.send_replace(PublicationStatus::Preparing);
            }
        }
        match result {
            Ok((store, snapshot, id, bytes, updates)) => {
                // The publisher thread is the only promoter. No request/lease
                // lock is held while SQLite writes the new durable authority.
                let commit = self.writer.commit_convoy_publication(
                    &observed,
                    (&id, &bytes),
                    &updates,
                    #[cfg(test)]
                    || super::test_sync::pause(&self.dirs.root.join("promotion")),
                );
                let published_revision = match commit {
                    Ok(Some(revision)) => revision,
                    failure => {
                        let config = self.config.read();
                        let mut state = self.state.lock();
                        state.promoting = false;
                        match failure {
                            Ok(None)
                            | Err(rusqlite::Error::SqliteFailure(
                                rusqlite::ffi::Error {
                                    code:
                                        rusqlite::ErrorCode::DatabaseBusy
                                        | rusqlite::ErrorCode::DatabaseLocked,
                                    ..
                                },
                                _,
                            )) => {
                                if matches!(mode, PublishMode::Rehash) {
                                    state.pending = Some(PublishMode::Rehash);
                                }
                            }
                            Err(err) if still_current(&state, &config) => {
                                tracing::warn!(%err, "Convoy publication commit failed");
                                *attempted = Some((observed, policy.clone(), ticket));
                                self.status.send_replace(PublicationStatus::Failed {
                                    reason: "commit_failed",
                                });
                            }
                            _ => (),
                        }
                        return;
                    }
                };
                // Even a vetoed commit owns its objects. Replace the old
                // durable snapshot (pins separately protect old bodies), but do
                // not admit it until this ticket, policy and stamp all agree.
                if let Some(owned) = &mut owned {
                    owned.commit();
                }
                let published_stamp = (published_revision, Some(id.clone()));
                let generation = snapshot.manifest.generation.clone();
                let bytes = snapshot.objects.values().map(|o| o.length).sum();
                let active = Active {
                    admissible: false,
                    revision: published_revision,
                    receipt: id,
                    policy: policy.clone(),
                    ticket,
                    snapshot: Arc::new(snapshot),
                    store,
                };
                let retired = {
                    let mut state = self.state.lock();
                    state.collected_dirty = true;
                    state.active.replace(active)
                };
                if let Some(retired) = retired {
                    let (live, available) = {
                        let mut state = self.state.lock();
                        let live: std::collections::BTreeSet<_> = state
                            .active
                            .as_ref()
                            .map(|active| active.snapshot.objects.keys().cloned().collect())
                            .unwrap_or_default();
                        state.retired.retain(|digest, _| !live.contains(digest));
                        (
                            live,
                            MAX_RETIRED_HANDLES.saturating_sub(state.retired.len()),
                        )
                    };
                    let mut held = Vec::new();
                    for (digest, record) in &retired.snapshot.objects {
                        if live.contains(digest) {
                            continue;
                        }
                        if held.len() == available {
                            tracing::warn!(
                                "retired Convoy handle limit reached; preserving remaining objects"
                            );
                            break;
                        }
                        match retired.store.lease(digest, record) {
                            Ok(file) => held.push((
                                digest.clone(),
                                (retired.store.clone(), file, record.clone()),
                            )),
                            Err(err) => {
                                tracing::warn!(%err, %digest, "cannot hold retired Convoy object; preserving it")
                            }
                        }
                    }
                    self.state.lock().retired.extend(held);
                }
                self.collect_retired();
                let stamp = self.capture_db.convoy_stamp();
                let config = self.config.read();
                let mut state = self.state.lock();
                state.promoting = false;
                if still_current(&state, &config) && stamp.ok().as_ref() == Some(&published_stamp) {
                    if let Some(active) = state.active.as_mut() {
                        active.admissible = true;
                    }
                    *attempted = Some((published_stamp, policy.clone(), ticket));
                    self.status
                        .send_replace(PublicationStatus::Ready { generation, bytes });
                }
            }
            Err(err) => {
                let config = self.config.read();
                let mut state = self.state.lock();
                if !still_current(&state, &config) {
                    if matches!(mode, PublishMode::Rehash) {
                        state.pending = Some(PublishMode::Rehash);
                    }
                    return;
                }
                *attempted = Some((observed, policy.clone(), ticket));
                let reason = if let Some(super::super::object_store::BudgetDeficit(deficit)) =
                    err.downcast_ref()
                {
                    if retired_pins.values().sum::<u64>() >= *deficit {
                        state.pin_wait = Some(PinWait {
                            mode,
                            deficit: *deficit,
                            retired_pins,
                        });
                        "pinned_capacity"
                    } else {
                        "store_capacity"
                    }
                } else {
                    "build_failed"
                };
                tracing::warn!(%err, "Convoy publication did not complete");
                self.status
                    .send_replace(PublicationStatus::Failed { reason });
            }
        }
    }
}

#[cfg(test)]
#[path = "promotion_tests.rs"]
mod promotion_tests;
