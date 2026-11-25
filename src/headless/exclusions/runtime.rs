//! Managed stopped-container boundary for the offline reconciler.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use super::{PolicyOperationGuard, PolicySnapshot, ReconcileReport};
use crate::{container::ContainerManager, dirs::QumaDirs};
use anyhow::{bail, Context, Result};
use bollard::models::ContainerInspectResponse;

/// Proof of inspected identity, not an authorization to start indefinitely later.
/// All lifecycle participants must hold the same guard and leave the filesystem
/// namespace stable. External `podman start` bypasses that cooperation contract.
#[derive(Debug, PartialEq, Eq)]
pub struct StoppedClient {
    id: String,
    backing: Vec<(PathBuf, u64, u64)>,
    resolved_merge: Option<PathBuf>,
}
impl StoppedClient {
    pub fn container_id(&self) -> &str {
        &self.id
    }
}

fn directory_identity(path: &Path) -> Result<(PathBuf, u64, u64)> {
    use std::os::unix::fs::MetadataExt;
    let absolute = std::path::absolute(path)?;
    let canonical = path.canonicalize()?;
    if absolute.as_os_str() != canonical.as_os_str() {
        bail!("aliased or noncanonical headless backing path");
    }
    let mut current = PathBuf::new();
    for component in absolute.components() {
        current.push(component);
        if std::fs::symlink_metadata(&current)?
            .file_type()
            .is_symlink()
        {
            bail!("symlinked headless backing ancestor");
        }
    }
    let m = std::fs::metadata(path)?;
    if !m.is_dir() {
        bail!("headless backing is not a directory");
    }
    Ok((canonical, m.dev(), m.ino()))
}

/// Podman records this descriptor after exit but may remove its entire numeric
/// overlay directory. It is not a durable backing layer and must never be made
/// to exist by inspection. Bind it to the inspected original container ID and
/// validate lexical shape, all existing ancestors and durable-layer separation.
fn resolved_merge_descriptor(
    source: &str,
    id: &str,
    backing: &[(PathBuf, u64, u64)],
) -> Result<PathBuf> {
    use std::os::unix::fs::MetadataExt;
    let path = Path::new(source);
    let components: Vec<_> = path.components().collect();
    let normalized: PathBuf = components.iter().collect();
    if !path.is_absolute()
        || normalized.as_os_str() != path.as_os_str()
        || components.iter().any(|c| {
            !matches!(
                c,
                std::path::Component::RootDir | std::path::Component::Normal(_)
            )
        })
        || components.len() < 7
    {
        bail!("unsupported resolved merge descriptor");
    }
    let names: Vec<_> = components.iter().map(|c| c.as_os_str()).collect();
    let tail = &names[names.len() - 6..];
    if tail[0] != "overlay-containers"
        || tail[1] != id
        || tail[2] != "userdata"
        || tail[3] != "overlay"
        || tail[5] != "merge"
        || !tail[4]
            .to_str()
            .is_some_and(|n| !n.is_empty() && n.bytes().all(|b| b.is_ascii_digit()))
    {
        bail!("resolved merge descriptor does not belong to original container");
    }
    if backing
        .iter()
        .any(|b| path.starts_with(&b.0) || b.0.starts_with(path))
    {
        bail!("conflicting resolved merge path");
    }
    let mut current = PathBuf::new();
    for component in components {
        current.push(component);
        match std::fs::symlink_metadata(&current) {
            Ok(m) if m.is_dir() && !m.file_type().is_symlink() => {
                if current == path && backing.iter().any(|b| (m.dev(), m.ino()) == (b.1, b.2)) {
                    bail!("resolved merge aliases a durable backing layer");
                }
            }
            Ok(_) => bail!("aliased or non-directory resolved merge ancestor"),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => break,
            Err(e) => return Err(e.into()),
        }
    }
    Ok(path.to_owned())
}

fn game_overlap(destination: &str) -> bool {
    let p = Path::new(destination);
    p.starts_with("/opt/tarkov") || Path::new("/opt/tarkov").starts_with(p)
}

fn validate_inspection(
    info: &ContainerInspectResponse,
    dirs: &QumaDirs,
    client: u32,
) -> Result<StoppedClient> {
    if client == 0 || dirs.is_legacy() {
        bail!("unsupported headless client/layout");
    }
    let expected_name = format!("fika-headless-{client}");
    if info.name.as_deref() != Some(&format!("/{expected_name}")) {
        bail!("unexpected headless container name");
    }
    let labels = info
        .config
        .as_ref()
        .and_then(|c| c.labels.as_ref())
        .context("missing managed labels")?;
    for (key, expected) in [
        ("managed-by", "quma"),
        ("quma.managed-by", "quartermaster-clients"),
        ("quma.client.index", &client.to_string()),
    ] {
        if labels.get(key).map(String::as_str) != Some(expected) {
            bail!("headless container identity label mismatch");
        }
    }
    let state = info.state.as_ref().context("unknown container state")?;
    if !matches!(
        state.status.as_ref().map(|s| s.as_ref()),
        Some("created" | "exited")
    ) || state.running != Some(false)
        || state.paused != Some(false)
        || state.restarting != Some(false)
        || state.pid != Some(0)
    {
        bail!("headless client requires a fully stopped created/exited container");
    }
    let id = info
        .id
        .as_ref()
        .filter(|s| !s.is_empty())
        .context("missing original container identity")?
        .clone();
    let backing = vec![
        directory_identity(&dirs.headless_base)?,
        directory_identity(&dirs.headless_upper(client))?,
        directory_identity(&dirs.headless_work(client))?,
    ];
    for (i, a) in backing.iter().enumerate() {
        for b in backing.iter().skip(i + 1) {
            if a.0.starts_with(&b.0) || b.0.starts_with(&a.0) || (a.1, a.2) == (b.1, b.2) {
                bail!("overlapping headless backing paths");
            }
        }
    }
    let host = info
        .host_config
        .as_ref()
        .context("missing native container bindings")?;
    if host.mounts.as_ref().is_some_and(|mounts| {
        mounts
            .iter()
            .any(|m| m.target.as_deref().is_none_or(game_overlap))
    }) {
        bail!("conflicting additional game mount");
    }
    let mut game = None;
    for bind in host.binds.as_ref().context("missing native game binding")? {
        let fields: Vec<_> = bind.split(':').collect();
        if fields.len() < 2 {
            bail!("unsupported container bind syntax");
        }
        if fields[1] != "/opt/tarkov" {
            if game_overlap(fields[1]) {
                bail!("overlapping game bind");
            }
            continue;
        }
        if fields.len() != 3 || game.is_some() {
            bail!("ambiguous game overlay binding");
        }
        game = Some((fields[0], fields[2]));
    }
    let (source, options) = game.context("missing original game overlay binding")?;
    let mut parsed = BTreeMap::new();
    for option in options.split(',') {
        let (key, value) = option.split_once('=').unwrap_or((option, ""));
        if parsed.insert(key, value).is_some() {
            bail!("duplicate overlay option");
        }
    }
    let raw = parsed.contains_key("O");
    let required = if raw {
        vec!["O", "upperdir", "workdir"]
    } else {
        vec!["lowerdir", "upperdir", "workdir", "private", "userxattr"]
    };
    if parsed.len() != required.len() || required.iter().any(|k| !parsed.contains_key(k)) {
        bail!("unsupported native game overlay options");
    }
    for flag in if raw {
        vec!["O"]
    } else {
        vec!["private", "userxattr"]
    } {
        if !parsed[flag].is_empty() {
            bail!("invalid native overlay flag");
        }
    }
    for (actual, expected) in [
        (if raw { source } else { parsed["lowerdir"] }, &backing[0]),
        (parsed["upperdir"], &backing[1]),
        (parsed["workdir"], &backing[2]),
    ] {
        if directory_identity(Path::new(actual))? != *expected {
            bail!("native headless overlay backing mismatch");
        }
    }
    let resolved_merge = if raw {
        None
    } else {
        Some(resolved_merge_descriptor(source, &id, &backing)?)
    };
    let mut seen = false;
    for mount in info.mounts.iter().flatten() {
        let destination = mount
            .destination
            .as_deref()
            .context("unknown native mount destination")?;
        if destination == "/opt/tarkov" {
            if seen
                || mount.source.as_deref() != Some(source)
                || mount.typ.as_deref() != Some("bind")
                || mount.rw != Some(true)
            {
                bail!("conflicting resolved native game mount");
            }
            seen = true;
        } else if game_overlap(destination) {
            bail!("overlapping native game mount");
        }
    }
    Ok(StoppedClient {
        id,
        backing,
        resolved_merge,
    })
}

/// Read-only inspect of the ORIGINAL managed container. Inspection errors refuse;
/// no discovery, creation or lifecycle control is performed.
pub async fn inspect_stopped_client(
    manager: &ContainerManager,
    dirs: &QumaDirs,
    client: u32,
) -> Result<StoppedClient> {
    let info = manager
        .inspect(&format!("fika-headless-{client}"))
        .await
        .map_err(|_| anyhow::anyhow!("cannot inspect original managed headless client"))?;
    validate_inspection(&info, dirs, client)
}

/// Check only: no policy directory/lock/mount/database writes or control calls.
pub async fn check_stopped_client(
    manager: &ContainerManager,
    dirs: &QumaDirs,
    policy: PolicySnapshot,
    client: u32,
) -> Result<ReconcileReport> {
    let before = inspect_stopped_client(manager, dirs, client).await?;
    let owned = dirs.clone();
    let report =
        tokio::task::spawn_blocking(move || super::check_private_overlay(&owned, &policy, client))
            .await??;
    let after = inspect_stopped_client(manager, dirs, client).await?;
    if before.container_id() != after.container_id()
        || before.backing != after.backing
        || before.resolved_merge != after.resolved_merge
    {
        bail!("headless context changed during check");
    }
    Ok(report)
}

/// Caller retains the Arc guard through any subsequent start. This never acquires
/// recursively. Snapshot loading must finish before calling this async boundary.
/// Any error MUST prevent start, even if an earlier journal action was applied.
pub async fn prepare_stopped_client(
    manager: &ContainerManager,
    dirs: &QumaDirs,
    policy: PolicySnapshot,
    client: u32,
    guard: Arc<PolicyOperationGuard>,
) -> Result<ReconcileReport> {
    guard.validate(dirs, client)?;
    let before = inspect_stopped_client(manager, dirs, client).await?;
    guard.validate(dirs, client)?;
    let owned = dirs.clone();
    let held = guard.clone();
    let report = tokio::task::spawn_blocking(move || {
        super::reconcile_private_overlay(&owned, &policy, client, &held)
    })
    .await??;
    guard.validate(dirs, client)?;
    let after = inspect_stopped_client(manager, dirs, client).await?;
    if before.container_id() != after.container_id()
        || before.backing != after.backing
        || before.resolved_merge != after.resolved_merge
    {
        bail!("headless context changed during reconciliation");
    }
    guard.validate(dirs, client)?;
    Ok(report)
}

/// Unforgeable outside this module: issued only after our actual create request
/// succeeded. It is single-use and bound to the very same held operation guard.
pub(crate) struct CreatedClient {
    stopped: StoppedClient,
    binds: Vec<String>,
    guard: Arc<PolicyOperationGuard>,
}

fn inspect_created(
    mut info: ContainerInspectResponse,
    dirs: &QumaDirs,
    client: u32,
    id: &str,
    binds: &[String],
) -> Result<StoppedClient> {
    if info.id.as_deref() != Some(id)
        || info
            .state
            .as_ref()
            .and_then(|s| s.status.as_ref())
            .map(|s| s.as_ref())
            != Some("created")
    {
        bail!("owned creation identity/state changed");
    }
    let host = info
        .host_config
        .as_mut()
        .context("missing owned create host configuration")?;
    // Only a never-started response with NO resolved bindings may use the
    // exact successful request. Populated or conflicting inspections stay strict.
    if host.binds.as_ref().is_none_or(Vec::is_empty)
        && host.mounts.as_ref().is_none_or(Vec::is_empty)
        && info.mounts.as_ref().is_none_or(Vec::is_empty)
    {
        host.binds = Some(binds.to_vec());
    }
    validate_inspection(&info, dirs, client)
}

pub(crate) async fn create_owned_client(
    manager: &ContainerManager,
    dirs: &QumaDirs,
    client: u32,
    opts: crate::container::CreateContainerOpts,
    guard: Arc<PolicyOperationGuard>,
) -> Result<CreatedClient> {
    guard.validate(dirs, client)?;
    let backing = vec![
        directory_identity(&dirs.headless_base)?,
        directory_identity(&dirs.headless_upper(client))?,
        directory_identity(&dirs.headless_work(client))?,
    ];
    if opts.name != format!("fika-headless-{client}") || opts.overlay_mounts.len() != 1 {
        bail!("invalid owned headless create request");
    }
    let overlay = &opts.overlay_mounts[0];
    for (path, expected) in [
        (&overlay.lower_dir, &backing[0]),
        (&overlay.upper_dir, &backing[1]),
        (&overlay.work_dir, &backing[2]),
    ] {
        if directory_identity(path)? != *expected {
            bail!("owned create backing mismatch");
        }
    }
    if opts.volumes.iter().any(|v| game_overlap(&v.container_path)) {
        bail!("owned create has overlapping game volume");
    }
    let mut binds: Vec<_> = opts.volumes.iter().map(|v| v.to_bind_string()).collect();
    binds.extend(opts.overlay_mounts.iter().map(|o| o.to_bind_string()));
    let id = manager.create_container(opts).await?;
    guard.validate(dirs, client)?;
    let stopped = inspect_created(manager.inspect(&id).await?, dirs, client, &id, &binds)?;
    if stopped.backing != backing {
        bail!("owned creation backing changed");
    }
    Ok(CreatedClient {
        stopped,
        binds,
        guard,
    })
}

/// Consume creation authority BEFORE the first configuration boot. Ordinary
/// stopped-container CLI/start inspections never receive this authority.
pub(crate) async fn prepare_created_client(
    manager: &ContainerManager,
    dirs: &QumaDirs,
    policy: PolicySnapshot,
    client: u32,
    created: CreatedClient,
) -> Result<()> {
    let CreatedClient {
        stopped,
        binds,
        guard,
    } = created;
    guard.validate(dirs, client)?;
    let before = inspect_created(
        manager.inspect(&stopped.id).await?,
        dirs,
        client,
        &stopped.id,
        &binds,
    )?;
    if before != stopped {
        bail!("owned creation context changed");
    }
    let owned = dirs.clone();
    let held = guard.clone();
    tokio::task::spawn_blocking(move || {
        super::reconcile_private_overlay(&owned, &policy, client, &held)
    })
    .await??;
    guard.validate(dirs, client)?;
    let after = inspect_created(
        manager.inspect(&stopped.id).await?,
        dirs,
        client,
        &stopped.id,
        &binds,
    )?;
    if after != stopped {
        bail!("owned creation context changed during preparation");
    }
    Ok(())
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use serde_json::json;

    fn fixture() -> (
        tempfile::TempDir,
        crate::dirs::QumaDirs,
        bollard::models::ContainerInspectResponse,
    ) {
        let tmp = tempfile::tempdir().unwrap();
        let dirs = crate::dirs::QumaDirs::from_root(tmp.path().to_owned());
        for p in [
            &dirs.headless_base,
            &dirs.headless_upper(1),
            &dirs.headless_work(1),
        ] {
            std::fs::create_dir_all(p).unwrap();
        }
        let bind = format!(
            "{}:/opt/tarkov:O,upperdir={},workdir={}",
            dirs.headless_base.display(),
            dirs.headless_upper(1).display(),
            dirs.headless_work(1).display()
        );
        let info = serde_json::from_value(json!({"Id":"original-id","Name":"/fika-headless-1", "Config":{"Labels":{"managed-by":"quma","quma.managed-by":"quartermaster-clients","quma.client.index":"1"}}, "State":{"Status":"exited","Running":false,"Paused":false,"Restarting":false,"Pid":0}, "HostConfig":{"Binds":[bind]}})).unwrap();
        (tmp, dirs, info)
    }

    #[test]
    fn stopped_resolved_merge_can_be_absent_without_creating_it() {
        let (_tmp, dirs, mut info) = fixture();
        let merge = dirs
            .root
            .join("storage/overlay-containers/original-id/userdata/overlay/123/merge");
        // Simulate Podman's clean stop: even the ephemeral numeric parent may
        // have gone, while the original durable backing directories remain.
        std::fs::create_dir_all(
            dirs.root
                .join("storage/overlay-containers/original-id/userdata/overlay"),
        )
        .unwrap();
        info.host_config.as_mut().unwrap().binds = Some(vec![format!(
            "{}:/opt/tarkov:lowerdir={},upperdir={},workdir={},private,userxattr",
            merge.display(),
            dirs.headless_base.display(),
            dirs.headless_upper(1).display(),
            dirs.headless_work(1).display()
        )]);
        for status in ["created", "exited"] {
            info.state.as_mut().unwrap().status =
                Some(serde_json::from_value(json!(status)).unwrap());
            super::validate_inspection(&info, &dirs, 1).unwrap();
            assert!(!merge.exists());
        }
    }

    #[test]
    fn absent_merge_descriptor_rejects_unsafe_aliases_and_conflicts() {
        let (_tmp, dirs, original) = fixture();
        let suffix = "overlay-containers/original-id/userdata/overlay/123/merge";
        let inspect_source = |source: &std::path::Path| {
            let mut info = original.clone();
            info.host_config.as_mut().unwrap().binds = Some(vec![format!(
                "{}:/opt/tarkov:lowerdir={},upperdir={},workdir={},private,userxattr",
                source.display(),
                dirs.headless_base.display(),
                dirs.headless_upper(1).display(),
                dirs.headless_work(1).display()
            )]);
            super::validate_inspection(&info, &dirs, 1)
        };
        for source in [
            dirs.root.join("unrecognized-merge"),
            dirs.root
                .join("storage/overlay-containers/wrong-id/userdata/overlay/123/merge"),
            dirs.root
                .join("storage/overlay-containers/original-id/userdata/overlay/not-numeric/merge"),
            dirs.headless_base.join(suffix),
            dirs.root.join("storage/../storage").join(suffix),
            std::path::PathBuf::from("relative").join(suffix),
        ] {
            assert!(
                inspect_source(&source).is_err(),
                "accepted {}",
                source.display()
            );
        }
        let merge = dirs.root.join("storage").join(suffix);
        std::fs::create_dir_all(merge.parent().unwrap()).unwrap();
        std::os::unix::fs::symlink(dirs.root.join("absent-target"), &merge).unwrap();
        assert!(inspect_source(&merge).is_err());
        std::fs::remove_file(&merge).unwrap();
        std::fs::write(&merge, b"not a directory").unwrap();
        assert!(inspect_source(&merge).is_err());
        std::fs::remove_file(&merge).unwrap();
        std::fs::rename(dirs.root.join("storage"), dirs.root.join("old-storage")).unwrap();
        std::os::unix::fs::symlink(dirs.root.join("old-storage"), dirs.root.join("storage"))
            .unwrap();
        assert!(inspect_source(&merge).is_err());
    }

    #[test]
    fn supported_raw_and_resolved_native_binds() {
        let (_tmp, dirs, mut info) = fixture();
        super::validate_inspection(&info, &dirs, 1).unwrap();
        info.state.as_mut().unwrap().status =
            Some(serde_json::from_value(json!("created")).unwrap());
        super::validate_inspection(&info, &dirs, 1).unwrap();
        let merged = dirs
            .root
            .join("storage/overlay-containers/original-id/userdata/overlay/123/merge");
        std::fs::create_dir_all(&merged).unwrap();
        info.host_config.as_mut().unwrap().binds = Some(vec![format!(
            "{}:/opt/tarkov:lowerdir={},upperdir={},workdir={},private,userxattr",
            merged.display(),
            dirs.headless_base.display(),
            dirs.headless_upper(1).display(),
            dirs.headless_work(1).display()
        )]);
        super::validate_inspection(&info, &dirs, 1).unwrap();
        info.mounts = Some(serde_json::from_value(json!([{"Type":"bind","Source":merged,"Destination":"/opt/tarkov","RW":true,"Propagation":"private"}])).unwrap());
        super::validate_inspection(&info, &dirs, 1).unwrap();
        info.mounts.as_mut().unwrap()[0].source = Some(dirs.root.to_string_lossy().into_owned());
        assert!(super::validate_inspection(&info, &dirs, 1).is_err());
    }

    #[test]
    fn invalid_identity_state_layout_and_aliases_refuse() {
        let (_tmp, dirs, info) = fixture();
        let value = serde_json::to_value(&info).unwrap();
        for (pointer, replacement) in [
            ("/Name", json!("/fika-headless-2")),
            ("/Config/Labels/managed-by", json!("other")),
            ("/Config/Labels/quma.client.index", json!("2")),
            ("/Config/Labels/quma.managed-by", json!("other")),
            ("/Id", json!("")),
            ("/State/Running", serde_json::Value::Null),
            ("/State/Status", json!("running")),
            ("/State/Status", json!("dead")),
            ("/State/Running", json!(true)),
            ("/State/Paused", json!(true)),
            ("/State/Restarting", json!(true)),
            ("/State/Pid", json!(123)),
            ("/State", serde_json::Value::Null),
            ("/HostConfig/Binds", json!([])),
        ] {
            let mut bad = value.clone();
            *bad.pointer_mut(pointer).unwrap() = replacement;
            let bad = serde_json::from_value(bad).unwrap();
            assert!(
                super::validate_inspection(&bad, &dirs, 1).is_err(),
                "accepted {pointer}"
            );
        }
        for target in ["lower", "upper", "work"] {
            let mut bad = info.clone();
            let mut lower = dirs.headless_base.clone();
            let mut upper = dirs.headless_upper(1);
            let mut work = dirs.headless_work(1);
            match target {
                "lower" => lower = dirs.root.clone(),
                "upper" => upper = dirs.root.clone(),
                _ => work = dirs.root.clone(),
            }
            bad.host_config.as_mut().unwrap().binds = Some(vec![format!(
                "{}:/opt/tarkov:O,upperdir={},workdir={}",
                lower.display(),
                upper.display(),
                work.display()
            )]);
            assert!(super::validate_inspection(&bad, &dirs, 1).is_err());
        }
        std::fs::rename(&dirs.headless_base, dirs.root.join("old-base")).unwrap();
        std::os::unix::fs::symlink(dirs.root.join("old-base"), &dirs.headless_base).unwrap();
        assert!(super::validate_inspection(&info, &dirs, 1).is_err());
    }
}
