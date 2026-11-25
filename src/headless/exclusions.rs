//! Validated headless file policy and private-overlay reconciliation.

pub(crate) mod lifecycle;
#[cfg(test)]
mod native_tests;
mod reconcile;
pub mod runtime;
#[cfg(test)]
pub(crate) mod test_support;
pub(crate) use reconcile::destroy_private_overlay;
pub(crate) use reconcile::resume_overlay_retirement;
pub use reconcile::{check_private_overlay, reconcile_private_overlay, ReconcileReport};

use std::collections::{BTreeMap, HashMap};
use std::path::PathBuf;

use anyhow::{bail, Context, Result};

use crate::db::Database;

/// Nonrecursive cross-process operation lock. Hold through preparation AND start.
/// The installation namespace must not be renamed/unlinked by out-of-band writers
/// while held. Identity checks detect replacement; they cannot stop hostile peers
/// running under the same Unix account. Never unlink persistent lock files.
#[derive(Debug)]
pub struct PolicyOperationGuard {
    file: std::fs::File,
    root_handle: cap_std::fs::Dir,
    state_handle: cap_std::fs::Dir,
    root: PathBuf,
    client: u32,
}

#[derive(Clone, Copy, PartialEq, Eq)]
struct Identity {
    dev: u64,
    ino: u64,
    uid: u32,
    mode: u32,
    nlink: u64,
}
impl From<std::fs::Metadata> for Identity {
    fn from(m: std::fs::Metadata) -> Self {
        use std::os::unix::fs::MetadataExt;
        Self {
            dev: m.dev(),
            ino: m.ino(),
            uid: m.uid(),
            mode: m.mode(),
            nlink: m.nlink(),
        }
    }
}
impl From<cap_std::fs::Metadata> for Identity {
    fn from(m: cap_std::fs::Metadata) -> Self {
        use cap_std::fs::MetadataExt;
        Self {
            dev: m.dev(),
            ino: m.ino(),
            uid: m.uid(),
            mode: m.mode(),
            nlink: m.nlink(),
        }
    }
}
fn same_inode(a: &Identity, b: &Identity) -> bool {
    a.dev == b.dev && a.ino == b.ino
}
fn private_metadata(metadata: &Identity, private: bool) -> Result<()> {
    // SAFETY: geteuid has no arguments or memory preconditions.
    if metadata.uid != unsafe { libc::geteuid() }
        || metadata.mode & if private { 0o077 } else { 0o022 } != 0
    {
        bail!("unsupported policy namespace ownership/permissions");
    }
    Ok(())
}

impl PolicyOperationGuard {
    /// Mutating operations only. Read-only checks must not acquire/create a lock.
    pub fn acquire(dirs: &crate::dirs::QumaDirs, client: u32) -> Result<Self> {
        use cap_std::fs::OpenOptionsExt;
        if client == 0 {
            bail!("headless client indices start at 1");
        }
        let root = dirs.root.canonicalize()?;
        if std::fs::symlink_metadata(&dirs.root)?
            .file_type()
            .is_symlink()
        {
            bail!("aliased policy root");
        }
        let root_handle = cap_std::fs::Dir::open_ambient_dir(&root, cap_std::ambient_authority())?;
        private_metadata(&root_handle.dir_metadata()?.into(), false)?;
        // Create private directories atomically with mode 0700 (not chmod of an
        // existing directory, whose unexpected permissions must be refused).
        use cap_std::fs::DirBuilderExt;
        let mut builder = cap_std::fs::DirBuilder::new();
        builder.mode(0o700);
        match root_handle.create_dir_with("headless-policy", &builder) {
            Ok(()) => (),
            Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => (),
            Err(e) => return Err(e.into()),
        }
        let meta = root_handle.symlink_metadata("headless-policy")?;
        if !meta.is_dir() || meta.file_type().is_symlink() {
            bail!("aliased policy state directory");
        }
        let state_handle = root_handle.open_dir("headless-policy")?;
        private_metadata(&state_handle.dir_metadata()?.into(), true)?;
        let name = format!("client-{client}.lock");
        let mut options = cap_std::fs::OpenOptions::new();
        options
            .read(true)
            .write(true)
            .create(true)
            .mode(0o600)
            .custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK);
        let file = state_handle.open_with(&name, &options)?.into_std();
        if !file.metadata()?.is_file() {
            bail!("operation lock is not a regular file");
        }
        private_metadata(&file.metadata()?.into(), true)?;
        file.try_lock()
            .context("headless policy operation is busy")?;
        let guard = Self {
            file,
            root_handle,
            state_handle,
            root,
            client,
        };
        guard.validate(dirs, client)?;
        Ok(guard)
    }

    /// Validate held inode identities, not just path strings, before guarded work.
    pub fn validate(&self, dirs: &crate::dirs::QumaDirs, client: u32) -> Result<()> {
        if self.client != client || self.root != dirs.root.canonicalize()? {
            bail!("policy operation guard belongs to another client");
        }
        let held_root: Identity = self.root_handle.dir_metadata()?.into();
        let named_root: Identity = std::fs::symlink_metadata(&dirs.root)?.into();
        let held_state: Identity = self.state_handle.dir_metadata()?.into();
        let named_state: Identity = self.root_handle.symlink_metadata("headless-policy")?.into();
        let held_file: Identity = self.file.metadata()?.into();
        let named_file: Identity = self
            .state_handle
            .symlink_metadata(format!("client-{client}.lock"))?
            .into();
        if named_root.mode & libc::S_IFMT != libc::S_IFDIR
            || named_state.mode & libc::S_IFMT != libc::S_IFDIR
            || named_file.mode & libc::S_IFMT != libc::S_IFREG
            || !same_inode(&held_root, &named_root)
            || !same_inode(&held_state, &named_state)
            || !same_inode(&held_file, &named_file)
            || held_file.nlink != 1
        {
            bail!("policy operation namespace was replaced");
        }
        private_metadata(&named_root, false)?;
        private_metadata(&named_state, true)?;
        private_metadata(&named_file, true)?;
        Ok(())
    }
}

/// File policy contains no credentials or container configuration.
#[derive(Debug, Clone)]
pub struct PolicyFile {
    pub excluded: bool,
    pub hash: Option<String>,
    pub size: Option<i64>,
}

/// A single SQLite read transaction captures group, mod, addon and file state.
#[derive(Debug, Clone)]
pub struct PolicySnapshot {
    files: BTreeMap<PathBuf, PolicyFile>,
}

impl PolicySnapshot {
    pub fn load(db: &Database) -> Result<Self> {
        let tx = db.begin_transaction()?;
        let groups: HashMap<_, _> = db
            .list_groups()?
            .into_iter()
            .map(|g| (g.id, g.exclude_headless))
            .collect();
        let mut mods = HashMap::new();
        for m in db.list_mods()? {
            let excluded = match m.group_id {
                Some(id) => *groups.get(&id).context("orphaned mod group membership")?,
                None => false,
            };
            mods.insert(m.id, excluded);
        }
        let addons: HashMap<_, _> = db
            .list_addons()?
            .into_iter()
            .map(|a| (a.id, a.parent_mod_id))
            .collect();
        let mut files: BTreeMap<PathBuf, PolicyFile> = BTreeMap::new();
        for file in db.get_all_tracked_files()? {
            // Rebuild actual validated components: PathBuf retains internal './'
            // and repeated separators in its string spelling.
            let path: PathBuf = crate::spt::scoped::relative_path(&file.file_path)?
                .components()
                .collect();
            let normalized = path.to_str().context("non-UTF8 policy path")?;
            if !crate::ops::is_client_file(normalized) {
                continue;
            }
            let owner = match (file.mod_id, file.addon_id) {
                (Some(id), None) => id,
                (None, Some(id)) => *addons.get(&id).context("orphaned addon file")?,
                _ => bail!("ambiguous file ownership"),
            };
            let excluded = *mods.get(&owner).context("orphaned file owner")?;
            if let Some(previous) = files.get(&path) {
                if previous.excluded != excluded
                    || previous.hash != file.file_hash
                    || previous.size != file.file_size
                {
                    bail!("conflicting owners for tracked client path");
                }
            } else {
                files.insert(
                    path,
                    PolicyFile {
                        excluded,
                        hash: file.file_hash,
                        size: file.file_size,
                    },
                );
            }
        }
        tx.commit()?;
        Ok(Self { files })
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use crate::db::Database;

    fn policy_db() -> (Database, i64) {
        let db = Database::open_in_memory().unwrap();
        let id = db
            .insert_mod(None, None, "EternalCycle", None, "1", "local", None, None)
            .unwrap();
        db.save_groups_atomic(&[(
            "No Headless".into(),
            "no-headless".into(),
            "optional".into(),
            true,
            vec![id],
        )])
        .unwrap();
        (db, id)
    }

    #[test]
    fn operation_guard_is_nonrecursive_and_survives_overlay_deletion() {
        let root = tempfile::tempdir().unwrap();
        let dirs = crate::dirs::QumaDirs::from_root(root.path().to_owned());
        std::fs::create_dir_all(dirs.headless_overlay(1)).unwrap();
        let guard = super::PolicyOperationGuard::acquire(&dirs, 1).unwrap();
        guard.validate(&dirs, 1).unwrap();
        assert!(guard.validate(&dirs, 2).is_err());
        assert!(super::PolicyOperationGuard::acquire(&dirs, 1).is_err());
        std::fs::remove_dir_all(dirs.headless_overlay(1)).unwrap();
        assert!(super::PolicyOperationGuard::acquire(&dirs, 1).is_err());
        drop(guard);
        // Other parallel tests may be between fork and exec and briefly inherit
        // this CLOEXEC fd. Only retry real busy, never namespace/permission errors.
        let mut acquired = None;
        for _ in 0..100 {
            match super::PolicyOperationGuard::acquire(&dirs, 1) {
                Ok(g) => {
                    acquired = Some(g);
                    break;
                }
                Err(e) if e.to_string().contains("busy") => {
                    std::thread::sleep(std::time::Duration::from_millis(10))
                }
                Err(e) => panic!("unexpected guard reacquisition failure: {e:#}"),
            }
        }
        assert!(acquired.is_some());
        assert!(super::PolicyOperationGuard::acquire(&dirs, 0).is_err());
    }

    #[test]
    fn normalized_config_and_server_aliases_are_preserved() {
        let (db, id) = policy_db();
        for path in [
            "BepInEx//config/a.cfg",
            "BepInEx/./config/b.cfg",
            "SPT//user/mods/a.json",
            "SPT_Runtime/./user/mods/b.json",
        ] {
            db.insert_file(id, path, None, None).unwrap();
        }
        let included = db
            .insert_mod(
                None,
                None,
                "Included config owner",
                None,
                "1",
                "local",
                None,
                None,
            )
            .unwrap();
        db.insert_file(included, "BepInEx/config/a.cfg", None, None)
            .unwrap();
        assert!(super::PolicySnapshot::load(&db).unwrap().files.is_empty());
    }

    #[test]
    fn canonical_and_dot_alias_policy_conflict_refuses() {
        let (db, id) = policy_db();
        let included = db
            .insert_mod(None, None, "Included", None, "1", "local", None, None)
            .unwrap();
        db.insert_file(id, "BepInEx/./plugins/a.dll", Some("abc"), Some(3))
            .unwrap();
        db.insert_file(included, "BepInEx/plugins/a.dll", Some("abc"), Some(3))
            .unwrap();
        assert!(super::PolicySnapshot::load(&db).is_err());
    }

    #[test]
    fn guard_rejects_replaced_namespace() {
        for replacement in ["root", "state", "lock"] {
            let tmp = tempfile::tempdir().unwrap();
            let path = tmp.path().join("root");
            std::fs::create_dir(&path).unwrap();
            let dirs = crate::dirs::QumaDirs::from_root(path.clone());
            let guard = super::PolicyOperationGuard::acquire(&dirs, 1).unwrap();
            match replacement {
                "root" => {
                    std::fs::rename(&path, tmp.path().join("old")).unwrap();
                    std::fs::create_dir(&path).unwrap();
                }
                "state" => {
                    std::fs::rename(path.join("headless-policy"), path.join("old")).unwrap();
                    std::fs::create_dir(path.join("headless-policy")).unwrap();
                }
                _ => {
                    std::fs::remove_file(path.join("headless-policy/client-1.lock")).unwrap();
                    std::fs::write(path.join("headless-policy/client-1.lock"), b"").unwrap();
                }
            }
            assert!(
                guard.validate(&dirs, 1).is_err(),
                "accepted replaced {replacement}"
            );
        }
    }

    #[test]
    fn guard_child_lock() {
        if let Some(root) = std::env::var_os("QUMA_TEST_GUARD_CHILD_ROOT") {
            let dirs = crate::dirs::QumaDirs::from_root(root.into());
            assert!(super::PolicyOperationGuard::acquire(&dirs, 1).is_err());
        }
    }

    #[test]
    fn guard_real_child_contention_and_unsafe_permissions() {
        use std::os::unix::fs::PermissionsExt;
        let tmp = tempfile::tempdir().unwrap();
        let dirs = crate::dirs::QumaDirs::from_root(tmp.path().to_owned());
        let guard = super::PolicyOperationGuard::acquire(&dirs, 1).unwrap();
        let status = std::process::Command::new(std::env::current_exe().unwrap())
            .args([
                "--exact",
                "headless::exclusions::tests::guard_child_lock",
                "--nocapture",
            ])
            .env("QUMA_TEST_GUARD_CHILD_ROOT", tmp.path())
            .output()
            .unwrap();
        assert!(status.status.success());
        std::fs::set_permissions(
            tmp.path().join("headless-policy/client-1.lock"),
            std::fs::Permissions::from_mode(0o666),
        )
        .unwrap();
        assert!(guard.validate(&dirs, 1).is_err());
        drop(guard);
        assert!(super::PolicyOperationGuard::acquire(&dirs, 1).is_err());
    }

    #[test]
    fn policy_snapshot_includes_addons_but_preserves_server_and_config() {
        let (db, id) = policy_db();
        db.insert_file(id, "BepInEx/plugins/EternalCycle.dll", Some("abc"), Some(3))
            .unwrap();
        db.insert_file(id, "BepInEx/config/EternalCycle.cfg", None, None)
            .unwrap();
        db.insert_file(id, "SPT/user/mods/EternalCycle/package.json", None, None)
            .unwrap();
        let addon = db.insert_addon(1, id, 1, "addon", None, "1", None).unwrap();
        db.insert_addon_file(addon, "BepInEx/plugins/addon.dll", Some("def"), Some(3))
            .unwrap();
        let policy = super::PolicySnapshot::load(&db).unwrap();
        assert_eq!(policy.files.len(), 2);
        assert!(policy.files.values().all(|file| file.excluded));
    }

    #[test]
    fn policy_snapshot_rejects_included_excluded_shared_owner() {
        let (db, id) = policy_db();
        let included = db
            .insert_mod(None, None, "Included", None, "1", "local", None, None)
            .unwrap();
        for (owner, path) in [
            (id, "BepInEx/plugins/shared.dll"),
            (included, r"BepInEx\plugins\shared.dll"),
        ] {
            db.insert_file(owner, path, Some("abc"), Some(3)).unwrap();
        }
        assert!(super::PolicySnapshot::load(&db).is_err());
    }

    #[test]
    fn policy_snapshot_rejects_unsafe_paths_and_missing_policy_schema() {
        let (db, id) = policy_db();
        db.insert_file(id, "../outside.dll", None, None).unwrap();
        assert!(super::PolicySnapshot::load(&db).is_err());
        db.conn().execute("DROP TABLE mod_groups", []).unwrap();
        assert!(super::PolicySnapshot::load(&db).is_err());
    }
}
