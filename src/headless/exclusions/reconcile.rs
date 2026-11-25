//! Reversible private overlay edits. The caller MUST keep the managed client stopped
//! under the operation guard through reconciliation and start. Mount detection is
//! an additional refusal gate, not a substitute for container lifecycle inspection.
mod proof;
use super::{PolicyOperationGuard, PolicySnapshot};
use crate::{
    dirs::QumaDirs,
    spt::scoped::{relative_path, ScopedDir},
};
use anyhow::{bail, Context, Result};
use cap_std::fs::{Dir, MetadataExt, OpenOptions, OpenOptionsExt};
use serde::{Deserialize, Serialize};
use std::{
    collections::BTreeMap,
    io::{Read, Write},
    path::{Path, PathBuf},
    process::Command,
};

#[derive(Debug, Serialize)]
pub struct ReconcileReport {
    pub client: u32,
    pub check_only: bool,
    pub actions: Vec<PathOutcome>,
    pub changed: usize,
    pub preserved: usize,
}
#[derive(Debug, Serialize)]
pub struct PathOutcome {
    pub path: PathBuf,
    pub outcome: String,
}
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
struct Identity {
    dev: u64,
    ino: u64,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
struct Entry {
    // Intent is durable before deletion. Original backups are never restoration sources.
    phase: String,
    original_hash: Option<String>,
    mask: Option<Identity>,
    restored_hash: Option<String>,
    #[serde(default)]
    previous_hash: Option<String>,
}
#[derive(Default, Serialize, Deserialize)]
struct Journal {
    version: u32,
    entries: BTreeMap<PathBuf, Entry>,
    #[serde(default)]
    generation: Option<Generation>,
    // Durable teardown intent; preparation refuses until teardown finishes.
    #[serde(default)]
    retiring: Option<Identity>,
}
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
struct Generation {
    overlay: Identity,
    upper: Identity,
    #[serde(default)]
    overlay_proof: Option<String>,
    #[serde(default)]
    upper_proof: Option<String>,
}
fn generation(dirs: &QumaDirs, client: u32) -> Result<Generation> {
    let root = Scope::open(&dirs.root)?;
    let identity = |path: PathBuf| -> Result<Identity> {
        Scope::open(&path)?;
        root.identity(path.strip_prefix(&dirs.root)?)?
            .context("overlay generation absent")
    };
    Ok(Generation {
        overlay: identity(dirs.headless_overlay(client))?,
        upper: identity(dirs.headless_upper(client))?,
        overlay_proof: proof::read(&Scope::open(&dirs.headless_overlay(client))?)?,
        upper_proof: proof::read(&Scope::open(&dirs.headless_upper(client))?)?,
    })
}
struct Scope {
    dir: Dir,
    scoped: ScopedDir,
}
impl Scope {
    fn open(path: &Path) -> Result<Self> {
        // Reject aliases in the root itself, not just its children.
        let mut probe = PathBuf::new();
        for c in path.components() {
            probe.push(c);
            if std::fs::symlink_metadata(&probe)?.file_type().is_symlink() {
                bail!("aliased scope root");
            }
        }
        Ok(Self {
            dir: Dir::open_ambient_dir(path, cap_std::ambient_authority())?,
            scoped: ScopedDir::open(path)?,
        })
    }
    fn validate(&self, path: &Path) -> Result<()> {
        let text = path.to_str().context("non UTF-8 path")?;
        if relative_path(text)? != path {
            bail!("noncanonical relative path");
        }
        self.scoped.require_unaliased_directories(path)
    }
    fn data(&self, path: &Path) -> Result<Option<Vec<u8>>> {
        self.validate(path)?;
        match self.scoped.read_overlay_file(path) {
            Ok(Some(mut f)) => {
                let mut b = Vec::new();
                f.read_to_end(&mut b)?;
                Ok(Some(b))
            }
            Ok(None) => Ok(None),
            Err(e)
                if e.downcast_ref::<std::io::Error>()
                    .is_some_and(|e| e.kind() == std::io::ErrorKind::NotFound) =>
            {
                Ok(None)
            }
            Err(e) => Err(e),
        }
    }
    fn identity(&self, path: &Path) -> Result<Option<Identity>> {
        self.validate(path)?;
        match self.dir.symlink_metadata(path) {
            Ok(m) => Ok(Some(Identity {
                dev: m.dev(),
                ino: m.ino(),
            })),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
            Err(e) => Err(e.into()),
        }
    }
    fn write_new(&self, path: &Path, bytes: &[u8]) -> Result<()> {
        self.validate(path)?;
        if let Some(p) = path.parent().filter(|p| !p.as_os_str().is_empty()) {
            self.dir.create_dir_all(p)?;
        }
        self.validate(path)?;
        let mut opts = OpenOptions::new();
        opts.write(true)
            .create_new(true)
            .mode(0o600)
            .custom_flags(libc::O_NOFOLLOW);
        let mut f = self.dir.open_with(path, &opts)?;
        f.write_all(bytes)?;
        f.sync_all()?;
        self.sync()
    }
    fn backup(&self, path: &Path, bytes: &[u8]) -> Result<()> {
        self.validate(path)?;
        if let Some(existing) = self.data(path)? {
            if existing != bytes {
                bail!("quarantine conflict");
            }
            return Ok(());
        }
        if let Some(p) = path.parent() {
            self.dir.create_dir_all(p)?;
        }
        static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
        let temporary = loop {
            let n = NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
            let p = PathBuf::from(format!("quarantine-partial-{}-{n}", std::process::id()));
            if self.identity(&p)?.is_none() {
                break p;
            }
        };
        // Interrupted partial files stay outside game view and are never trusted.
        self.write_new(&temporary, bytes)?;
        self.dir.hard_link(&temporary, &self.dir, path)?;
        if let Some(p) = path.parent() {
            self.dir.open_dir(p)?.open(".")?.sync_all()?;
        }
        self.dir.remove_file(temporary)?;
        self.sync()
    }
    fn sync(&self) -> Result<()> {
        self.dir.open(".")?.sync_all()?;
        Ok(())
    }
}
fn restore_temp(path: &Path) -> PathBuf {
    path.with_file_name(format!(
        ".quma-exclusion-restore-{}",
        hash(path.as_os_str().as_encoded_bytes())
    ))
}
fn hash(b: &[u8]) -> String {
    crate::spt::mods::compute_hash_public(b)
}
fn state_path(client: u32) -> PathBuf {
    PathBuf::from(format!("headless-policy/client-{client}"))
}
fn load(root: &Scope, client: u32) -> Result<Journal> {
    let p = state_path(client).join("journal.json");
    root.validate(&p)?;
    match root.data(&p)? {
        Some(b) => {
            let j: Journal = serde_json::from_slice(&b)?;
            if !matches!(j.version, 1..=3)
                || (j.version == 1 && (j.generation.is_some() || j.retiring.is_some()))
            {
                bail!("unsupported policy journal");
            }
            if j.version < 3
                && (j.generation.is_some()
                    || j.retiring.is_some()
                    || j.entries
                        .values()
                        .any(|e| e.phase != "retired" || e.mask.is_some()))
            {
                bail!("legacy policy ownership lacks durable generation proof");
            }
            if j.version == 3
                && j.generation.as_ref().is_some_and(|g| {
                    !g.overlay_proof.as_deref().is_some_and(proof::valid)
                        || !g.upper_proof.as_deref().is_some_and(proof::valid)
                })
            {
                bail!("missing or malformed policy generation proof");
            }
            if j.version >= 2
                && ((j.generation.is_none()
                    && (j.retiring.is_some() || j.entries.values().any(|e| e.phase != "retired")))
                    || j.entries
                        .values()
                        .any(|e| e.phase == "retired" && e.mask.is_some())
                    || j.retiring
                        .as_ref()
                        .is_some_and(|id| j.generation.as_ref().is_none_or(|g| &g.overlay != id)))
            {
                bail!("inconsistent policy generation metadata");
            }
            Ok(j)
        }
        None => {
            if root.identity(&p)?.is_some() {
                bail!("journal is masked/nonregular");
            }
            Ok(Journal {
                version: 1,
                ..Journal::default()
            })
        }
    }
}
fn save(state: &Scope, j: &Journal) -> Result<()> {
    let tmp = Path::new("journal.next");
    // A leftover temporary file can only be replaced after validating it is regular.
    if state.identity(tmp)?.is_some() {
        state.data(tmp)?.context("invalid journal temporary file")?;
        state.dir.remove_file(tmp)?;
    }
    state.write_new(tmp, &serde_json::to_vec(j)?)?;
    state.dir.rename(tmp, &state.dir, "journal.json")?;
    state.sync()
}

/// Destroy only an explicitly selected, stopped/removed client's durable overlay.
/// The caller holds the lifecycle guard and has removed its container. Journals
/// and lock inodes remain outside the teardown; interrupted work requires retry.
pub(crate) fn destroy_private_overlay(
    dirs: &QumaDirs,
    client: u32,
    guard: &PolicyOperationGuard,
) -> Result<()> {
    destroy_private_overlay_with(
        dirs,
        client,
        guard,
        || dirs.headless_overlay_mount(client).unmount(),
        crate::overlay::remove_unused_overlay_dir,
    )
}

/// Resume only a previously durable teardown authorization, never an inference
/// from an absent container or changed upper. Ordinary preparation still refuses.
pub(crate) fn resume_overlay_retirement(
    dirs: &QumaDirs,
    client: u32,
    guard: &PolicyOperationGuard,
) -> Result<()> {
    guard.validate(dirs, client)?;
    if load(&Scope::open(&dirs.root)?, client)?.retiring.is_some() {
        destroy_private_overlay(dirs, client, guard)?;
    }
    Ok(())
}

fn destroy_private_overlay_with(
    dirs: &QumaDirs,
    client: u32,
    guard: &PolicyOperationGuard,
    unmount: impl FnOnce() -> Result<()>,
    remove: impl FnOnce(&Path) -> Result<()>,
) -> Result<()> {
    guard.validate(dirs, client)?;
    if dirs.is_legacy() {
        bail!("private overlay teardown requires managed directory layout");
    }
    let root = Scope::open(&dirs.root)?;
    let overlay = dirs.headless_overlay(client);
    let relative = overlay.strip_prefix(&dirs.root)?;
    root.validate(relative)?;
    let named = root.identity(relative)?;
    let mut j = load(&root, client)?;
    if let Some(pending) = &j.retiring {
        if named.as_ref().is_some_and(|id| id != pending) {
            bail!("retiring overlay namespace changed");
        }
        if named.is_some() {
            proof::verify_retiring(
                dirs,
                client,
                j.generation
                    .as_ref()
                    .context("missing retiring generation")?,
            )?;
        }
    } else if named.is_some() {
        let current = generation(dirs, client)?;
        if j.generation.as_ref().is_some_and(|old| old != &current) {
            bail!("cannot destroy an unrecognized overlay generation");
        }
        j.generation = Some(if j.generation.is_none() {
            proof::initialize_generation(dirs, client)?
        } else {
            current
        });
        j.version = 3;
    } else {
        if j.generation.is_some() || j.entries.values().any(|e| e.phase != "retired") {
            bail!("owned overlay disappeared without retirement intent");
        }
        return Ok(());
    }
    // Reject aliases even on a partial retry, and never recurse into a mount.
    for path in [dirs.headless_upper(client), dirs.headless_work(client)] {
        root.validate(path.strip_prefix(&dirs.root)?)?;
    }
    unmount().context("failed to unmount retiring client overlay")?;
    refuse_busy(dirs, client)?;
    let canonical_overlay = dirs.root.canonicalize()?.join(relative);
    for line in std::fs::read_to_string("/proc/self/mountinfo")?.lines() {
        let mounted = line
            .split_whitespace()
            .nth(4)
            .context("invalid mountinfo")?;
        // Own root spelling was checked by refuse_busy, so escaped descendant
        // names still retain this exact unescaped root prefix and separator.
        if Path::new(mounted).starts_with(&canonical_overlay) {
            bail!("retiring overlay contains an active mount");
        }
    }
    let rel = state_path(client);
    root.validate(&rel)?;
    use cap_std::fs::DirBuilderExt;
    let mut builder = cap_std::fs::DirBuilder::new();
    builder.mode(0o700);
    match root.dir.create_dir_with(&rel, &builder) {
        Ok(()) => {
            root.dir
                .open_dir("headless-policy")?
                .open(".")?
                .sync_all()?;
            root.sync()?;
        }
        Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => (),
        Err(e) => return Err(e.into()),
    }
    let state = Scope::open(&dirs.root.join(&rel))?;
    // Legacy client directories can be 0755 inside the validated 0700 namespace.
    super::private_metadata(&state.dir.dir_metadata()?.into(), false)?;
    if j.retiring.is_none() {
        // Content-addressed immutable evidence is durable before destructive intent.
        let bytes = serde_json::to_vec(&j)?;
        let evidence = PathBuf::from(format!("retired/journal-{}.json", hash(&bytes)));
        state.backup(&evidence, &bytes)?;
        j.retiring = named.clone();
        j.version = 3;
        save(&state, &j)?;
    }
    guard.validate(dirs, client)?;
    if root.identity(relative)? != named {
        bail!("overlay changed during retirement");
    }
    if named.is_some() {
        proof::verify_retiring(
            dirs,
            client,
            j.generation
                .as_ref()
                .context("missing retiring generation")?,
        )?;
        remove(&overlay)?;
        if root.identity(relative)?.is_some() {
            bail!("overlay removal did not finish");
        }
    }
    root.dir
        .open_dir(relative.parent().context("missing overlay parent")?)?
        .open(".")?
        .sync_all()?;
    // Keep generation-independent intent/hashes, never native inode authority.
    for e in j.entries.values_mut() {
        e.phase = "retired".into();
        e.mask = None;
    }
    j.generation = None;
    j.retiring = None;
    save(&state, &j)
}

fn refuse_busy(dirs: &QumaDirs, client: u32) -> Result<()> {
    // A retry of a partially removed generation may have lost either directory.
    let resolve = |p: PathBuf| -> Result<PathBuf> {
        match p.canonicalize() {
            Ok(p) => Ok(p),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(p),
            Err(e) => Err(e.into()),
        }
    };
    let upper = resolve(dirs.headless_upper(client))?;
    let work = resolve(dirs.headless_work(client))?;
    // mountinfo escapes spaces/backslashes; reject ambiguous option delimiters up front.
    for p in [&upper, &work, &dirs.headless_base] {
        if p.to_string_lossy()
            .chars()
            .any(|c| matches!(c, ':' | ',' | '\\' | '\n' | ' '))
        {
            bail!("unsupported overlay directory spelling");
        }
    }
    let mounts = std::fs::read_to_string("/proc/self/mountinfo")?;
    for line in mounts.lines() {
        if line.contains(&format!("upperdir={}", upper.display()))
            || line.contains(&format!("workdir={}", work.display()))
        {
            bail!("private overlay is mounted/busy");
        }
    }
    // FUSE mountinfo does not expose backing directories. Its daemon command
    // line does; inspect visible daemons as well as kernel overlay mount options.
    for proc in std::fs::read_dir("/proc")? {
        let proc = proc?;
        if !proc
            .file_name()
            .to_string_lossy()
            .bytes()
            .all(|b| b.is_ascii_digit())
        {
            continue;
        }
        let cmd = match std::fs::read(proc.path().join("cmdline")) {
            Ok(cmd) => cmd,
            Err(e)
                if matches!(
                    e.kind(),
                    std::io::ErrorKind::NotFound | std::io::ErrorKind::PermissionDenied
                ) || e.raw_os_error() == Some(libc::ESRCH) =>
            {
                continue
            }
            Err(e) => return Err(e.into()),
        };
        let text = String::from_utf8_lossy(&cmd);
        if text.contains("fuse-overlayfs")
            && text.contains(&format!("upperdir={}", upper.display()))
        {
            bail!("private overlay has an active FUSE daemon");
        }
    }
    // Never open or clear Podman's kernel work/work (often mode 000).
    // Refuse known volatile state rather than attempting recovery of its workdir.
    let lock = dirs.headless_work(client).join("work/incompat/volatile");
    if lock.exists() {
        bail!("volatile overlay work state is unsupported");
    }
    Ok(())
}
#[derive(Clone)]
struct Action {
    path: PathBuf,
    kind: &'static str,
    bytes: Option<Vec<u8>>,
    upper: Option<Vec<u8>>,
    // Expected native merged bytes, distinct from physical upper ownership.
    // Populated for restoration only after lower/mask authorization in preflight.
    destination: Option<Vec<u8>>,
}
fn restoration_destination(
    upper: &Scope,
    lower: &Scope,
    path: &Path,
    up: &Option<Vec<u8>>,
    entry: &Entry,
    current: &[u8],
) -> Result<Option<Vec<u8>>> {
    if up.is_some() {
        return Ok(up.clone());
    }
    if let Some(id) = upper.identity(path)? {
        // Only an owned completed mask or the result of our durable unlink
        // intent may be replaced. An unrelated later mask is not ours to clear.
        if entry.mask.as_ref() != Some(&id) && entry.phase != "hide-intent" {
            bail!("unowned restoration mask");
        }
    }
    if upper.scoped.blocks_lower_layer(path)? {
        return Ok(None);
    }
    let destination = lower.data(path)?;
    if destination.is_none() && lower.identity(path)?.is_some() {
        bail!("unsupported restoration lower entry");
    }
    if let Some(bytes) = &destination {
        let h = hash(bytes);
        if bytes != current
            && ![
                entry.original_hash.as_ref(),
                entry.restored_hash.as_ref(),
                entry.previous_hash.as_ref(),
            ]
            .into_iter()
            .flatten()
            .any(|known| known == &h)
        {
            bail!("unverifiable restoration lower payload");
        }
    }
    Ok(destination)
}
fn plan(dirs: &QumaDirs, policy: &PolicySnapshot, client: u32) -> Result<(Journal, Vec<Action>)> {
    if client == 0 {
        bail!("headless client indices start at 1");
    }
    if dirs.is_legacy() {
        bail!("private headless reconciliation requires the managed directory layout");
    }
    let backing = [
        dirs.headless_base.canonicalize()?,
        dirs.headless_upper(client).canonicalize()?,
        dirs.headless_work(client).canonicalize()?,
    ];
    for (i, a) in backing.iter().enumerate() {
        for b in backing.iter().skip(i + 1) {
            if a.starts_with(b) || b.starts_with(a) {
                bail!("overlay backing directories overlap");
            }
        }
        let state = dirs.root.canonicalize()?.join(state_path(client));
        if state.starts_with(a) || a.starts_with(&state) {
            bail!("policy state overlaps overlay backing");
        }
    }
    let root = Scope::open(&dirs.root)?;
    root.validate(&state_path(client))?;
    let upper = Scope::open(&dirs.headless_upper(client))?;
    let lower = Scope::open(&dirs.headless_base)?;
    Scope::open(&dirs.headless_work(client))?;
    refuse_busy(dirs, client)?;
    let j = load(&root, client)?;
    if j.retiring.is_some() {
        bail!("overlay generation retirement incomplete; retry authorized teardown");
    }
    let current = generation(dirs, client)?;
    if j.generation.as_ref().is_some_and(|old| old != &current) {
        bail!("policy overlay generation changed without authorized retirement");
    }
    let mut paths = policy
        .files
        .keys()
        .chain(j.entries.keys())
        .cloned()
        .collect::<Vec<_>>();
    paths.sort();
    paths.dedup();
    let mut actions = Vec::new();
    for path in paths {
        upper.validate(&path)?;
        lower.validate(&path)?;
        if !crate::ops::is_client_file(path.to_str().context("non UTF8 policy")?) {
            bail!("journal/policy targets non-client file");
        }
        let up = upper.data(&path)?;
        let id = upper.identity(&path)?;
        let owned = j.entries.get(&path);
        let p = policy.files.get(&path);
        let excluded = p.is_none_or(|p| p.excluded);
        if let Some(e) = owned {
            if !matches!(
                e.phase.as_str(),
                "hide-intent" | "hidden" | "restore-intent" | "included" | "retired"
            ) {
                bail!("invalid journal phase");
            }
            if e.phase == "hidden" && e.mask != id {
                bail!("owned hidden state changed: {}", path.display());
            }
            if let Some(b) = &up {
                let h = hash(b);
                if ![
                    e.original_hash.as_ref(),
                    e.restored_hash.as_ref(),
                    e.previous_hash.as_ref(),
                ]
                .into_iter()
                .flatten()
                .any(|v| v == &h)
                {
                    bail!("owned payload modified");
                }
            }
        }
        let kind;
        let mut bytes = None;
        let mut destination = None;
        if excluded {
            if let Some(b) = &up {
                let source = if let Some(policy_file) = p {
                    published(dirs, &path, policy_file)?
                } else {
                    b.clone()
                };
                if *b != source {
                    bail!("modified/unverifiable upper: {}", path.display());
                }
                bytes = Some(source);
                kind = "hide";
            } else if owned.is_some_and(|e| e.phase == "hidden")
                && (id.is_some() || lower.data(&path)?.is_none())
            {
                kind = "retain-hidden";
            } else if owned.is_some_and(|e| e.phase != "retired")
                && (id.is_some() || lower.data(&path)?.is_none())
            {
                kind = "recover-hidden";
            } else if (owned.is_none() || owned.is_some_and(|e| e.phase == "retired"))
                && (id.is_some() || upper.scoped.blocks_lower_layer(&path)?)
            {
                kind = "preserve-unowned-mask";
            } else if lower.data(&path)?.is_some() {
                let observed = lower.data(&path)?.context("lower disappeared")?;
                let source = if let Some(pf) = p {
                    published(dirs, &path, pf)?
                } else {
                    if owned.and_then(|e| e.original_hash.as_ref()) != Some(&hash(&observed)) {
                        bail!("retired lower changed");
                    }
                    observed.clone()
                };
                if observed != source {
                    bail!("unverifiable lower payload");
                }
                bytes = Some(source);
                kind = "hide";
            } else {
                kind = "already-absent";
            }
        } else if let Some(e) = owned {
            let source = published(dirs, &path, p.context("included policy disappeared")?)?;
            if up.as_ref() == Some(&source) {
                kind = if e.phase == "included" {
                    "already-included"
                } else {
                    "recover-included"
                };
            } else {
                kind = "restore-current";
                destination = restoration_destination(&upper, &lower, &path, &up, e, &source)?;
                bytes = Some(source);
            }
        } else {
            kind = "preserve-included";
        }
        if kind == "restore-current" {
            let temporary = restore_temp(&path);
            upper.validate(&temporary)?;
            lower.validate(&temporary)?;
            if lower.identity(&temporary)?.is_some()
                || (upper.identity(&temporary)?.is_some()
                    && !owned.is_some_and(|e| e.phase == "restore-intent"))
            {
                bail!("restoration temporary path conflict");
            }
        }
        let backup = state_path(client).join("quarantine").join(&path);
        root.validate(&backup)?;
        if let Some(b) = &up {
            let backup = backup.join(hash(b));
            root.validate(&backup)?;
            if let Some(existing) = root.data(&backup)? {
                if existing != *b {
                    bail!("quarantine conflict");
                }
            }
        }
        actions.push(Action {
            path,
            kind,
            bytes,
            upper: up,
            destination,
        });
    }
    Ok((j, actions))
}
fn published(dirs: &QumaDirs, path: &Path, p: &super::PolicyFile) -> Result<Vec<u8>> {
    let text = path.to_str().context("non UTF8 path")?;
    let resolved = dirs.resolve_mod_file(text);
    let roots = [
        dirs.runtime_upper(),
        dirs.mod_overlay(),
        dirs.spt_server.clone(),
    ];
    let root = roots
        .iter()
        .find(|root| resolved == root.join(path))
        .context("unrecognized published source")?;
    let b = Scope::open(root)?
        .data(path)?
        .context("published source absent or masked")?;
    if p.hash.as_ref() != Some(&hash(&b)) || p.size != Some(i64::try_from(b.len())?) {
        bail!("published source fails tracked hash/size verification");
    }
    Ok(b)
}
fn report(client: u32, check_only: bool, actions: &[Action]) -> ReconcileReport {
    let changed = actions
        .iter()
        .filter(|a| matches!(a.kind, "hide" | "restore-current"))
        .count();
    ReconcileReport {
        client,
        check_only,
        changed,
        preserved: actions.len() - changed,
        actions: actions
            .iter()
            .map(|a| PathOutcome {
                path: a.path.clone(),
                outcome: if check_only {
                    format!("planned:{}", a.kind)
                } else {
                    a.kind.into()
                },
            })
            .collect(),
    }
}
/// Read-only inspection; never creates directories, acquires locks or mounts.
pub fn check_private_overlay(
    dirs: &QumaDirs,
    policy: &PolicySnapshot,
    client: u32,
) -> Result<ReconcileReport> {
    let (_, a) = plan(dirs, policy, client)?;
    Ok(report(client, true, &a))
}
struct Mounted {
    path: PathBuf,
    temp: Option<tempfile::TempDir>,
    active: bool,
}
impl Mounted {
    fn new(dirs: &QumaDirs, client: u32, state: &Path) -> Result<Self> {
        let temp = tempfile::Builder::new()
            .prefix("native-")
            .tempdir_in(state)?;
        let path = temp.path().join("merged");
        let work = temp.path().join("work");
        std::fs::create_dir(&path)?;
        std::fs::create_dir(&work)?;
        use std::os::unix::fs::MetadataExt;
        if std::fs::metadata(&work)?.dev() != std::fs::metadata(dirs.headless_upper(client))?.dev()
        {
            bail!("FUSE work and upper must share filesystem");
        }
        let options = format!(
            "lowerdir={},upperdir={},workdir={},squash_to_uid={},squash_to_gid={}",
            dirs.headless_base.display(),
            dirs.headless_upper(client).display(),
            work.display(),
            unsafe { libc::geteuid() },
            unsafe { libc::getegid() }
        );
        let output = Command::new("fuse-overlayfs")
            .args(["-o", &options])
            .arg(&path)
            .output()?;
        if !output.status.success() {
            bail!(
                "native FUSE mount failed: {}",
                String::from_utf8_lossy(&output.stderr)
            );
        }
        Ok(Self {
            path,
            temp: Some(temp),
            active: true,
        })
    }
    fn unmount(&mut self) -> Result<()> {
        // Other threads may fork while a scoped merged fd is open. Its child
        // closes CLOEXEC descriptors at exec, so retry transient EBUSY only.
        // Never lazy-unmount: a real busy view must fail closed and retain work.
        for attempt in 0..100 {
            let o = Command::new("fusermount3")
                .arg("-u")
                .arg(&self.path)
                .output()?;
            if o.status.success() {
                self.active = false;
                return Ok(());
            }
            let error = String::from_utf8_lossy(&o.stderr);
            if !error.contains("Device or resource busy") || attempt == 99 {
                bail!("FUSE unmount failed: {error}");
            }
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
        unreachable!()
    }
}
impl Drop for Mounted {
    fn drop(&mut self) {
        if self.active && self.unmount().is_err() {
            // Never recursively clean an active mount or its backing workdir.
            if let Some(t) = self.temp.take() {
                let _ = t.keep();
            }
        }
    }
}
/// Apply only to a stopped managed client. The adapter MUST inspect stopped state
/// while holding this same guard, and MUST NOT start after any returned error.
pub fn reconcile_private_overlay(
    dirs: &QumaDirs,
    policy: &PolicySnapshot,
    client: u32,
    guard: &PolicyOperationGuard,
) -> Result<ReconcileReport> {
    guard.validate(dirs, client)?;
    let (mut j, actions) = plan(dirs, policy, client)?;
    let root = Scope::open(&dirs.root)?;
    let rel = state_path(client);
    root.validate(&rel)?;
    root.dir.create_dir_all(&rel)?;
    // The journal directory's name must survive a crash before native edits.
    root.dir
        .open_dir("headless-policy")?
        .open(".")?
        .sync_all()?;
    root.sync()?;
    let state_abs = dirs.root.join(&rel);
    let state = Scope::open(&state_abs)?;
    let upper = Scope::open(&dirs.headless_upper(client))?;
    // Only an unbound journal can initialize proof; never repair owned metadata.
    j.version = 3;
    if j.generation.is_none() {
        j.generation = Some(proof::initialize_generation(dirs, client)?);
    }
    save(&state, &j)?;
    // Validate the entire plan (including backup paths) before any destructive action.
    for a in &actions {
        state.validate(&Path::new("quarantine").join(&a.path))?;
        if let Some(b) = &a.upper {
            let backup = Path::new("quarantine").join(&a.path).join(hash(b));
            state.validate(&backup)?;
            if let Some(existing) = state.data(&backup)? {
                if existing != *b {
                    bail!("quarantine conflict");
                }
            }
        }
    }
    for a in &actions {
        if matches!(a.kind, "recover-hidden" | "recover-included") {
            refuse_busy(dirs, client)?;
            let mut verify = Mounted::new(dirs, client, &state_abs)?;
            let visible = Scope::open(&verify.path)?.data(&a.path)?;
            if (a.kind == "recover-hidden" && visible.is_some())
                || (a.kind == "recover-included" && visible != a.upper)
            {
                bail!("interrupted state visibility conflict");
            }
            verify.unmount()?;
            let e = j
                .entries
                .get_mut(&a.path)
                .context("missing recovery entry")?;
            e.phase = if a.kind == "recover-hidden" {
                "hidden"
            } else {
                "included"
            }
            .into();
            e.mask = if a.kind == "recover-hidden" {
                upper.identity(&a.path)?
            } else {
                None
            };
            save(&state, &j)?;
            continue;
        }
        if !matches!(a.kind, "hide" | "restore-current") {
            continue;
        }
        refuse_busy(dirs, client)?;
        if a.kind == "hide" {
            if let Some(original) = &a.upper {
                let backup = Path::new("quarantine").join(&a.path).join(hash(original));
                state.validate(&backup)?;
                state.backup(&backup, original)?;
                if state.data(&backup)?.as_ref() != Some(original) {
                    bail!("quarantine verification failed");
                }
            }
            j.entries.insert(
                a.path.clone(),
                Entry {
                    phase: "hide-intent".into(),
                    original_hash: a.bytes.as_ref().map(|b| hash(b)),
                    mask: None,
                    restored_hash: None,
                    previous_hash: None,
                },
            );
        } else {
            let e = j.entries.get_mut(&a.path).context("missing owned state")?;
            e.phase = "restore-intent".into();
            // Preserve authorization of the effective prior destination across
            // interruption, including a previously verified visible lower file.
            e.previous_hash = a.destination.as_ref().map(|b| hash(b));
            e.restored_hash = a.bytes.as_ref().map(|b| hash(b));
        }
        save(&state, &j)?;
        let mut mount = Mounted::new(dirs, client, &state_abs)?;
        let view = Scope::open(&mount.path)?;
        if a.kind == "hide" {
            let visible = view.data(&a.path)?;
            if visible.as_ref() != a.bytes.as_ref() {
                bail!("merged payload changed before unlink");
            }
            view.validate(&a.path)?;
            view.dir.remove_file(&a.path)?;
            if let Some(parent) = a.path.parent() {
                view.dir.open_dir(parent)?.open(".")?.sync_all()?;
            }
        } else {
            view.validate(&a.path)?;
            if let Some(parent) = a.path.parent() {
                view.dir.create_dir_all(parent)?;
            }
            // Verify the explicitly authorized merged destination, not merely
            // physical upper bytes: a verified lower may legitimately be visible.
            let visible = view.data(&a.path)?;
            if visible != a.destination {
                bail!("restore destination changed");
            }
            let payload = a.bytes.as_deref().context("missing restoration bytes")?;
            let temporary = restore_temp(&a.path);
            view.validate(&temporary)?;
            if view.identity(&temporary)?.is_some() {
                // This name was reserved in the durable restore intent. A partial
                // previous write is safe to discard, never a user target.
                view.dir.remove_file(&temporary)?;
            }
            view.write_new(&temporary, payload)?;
            view.dir.rename(&temporary, &view.dir, &a.path)?;
            if let Some(parent) = a.path.parent() {
                view.dir.open_dir(parent)?.open(".")?.sync_all()?;
            }
        }
        drop(view);
        mount.unmount()?;
        // Verify native semantics across a completely new mount.
        let mut verify = Mounted::new(dirs, client, &state_abs)?;
        let observed = Scope::open(&verify.path)?.data(&a.path)?;
        if (a.kind == "hide" && observed.is_some())
            || (a.kind == "restore-current" && observed != a.bytes)
        {
            bail!("remount visibility verification failed");
        }
        verify.unmount()?;
        let e = j.entries.get_mut(&a.path).context("lost journal entry")?;
        e.phase = if a.kind == "hide" {
            "hidden"
        } else {
            "included"
        }
        .into();
        e.mask = if a.kind == "hide" {
            upper.identity(&a.path)?
        } else {
            None
        };
        save(&state, &j)?;
    }
    Ok(report(client, false, &actions))
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests;
