use anyhow::{ensure, Context, Result};
use cap_std::fs::{Dir, OpenOptions};

use std::{
    collections::BTreeMap,
    fs::{self, File, Metadata},
    io::{Read, Write},
    os::{
        fd::AsRawFd,
        unix::fs::{DirBuilderExt, MetadataExt, PermissionsExt},
    },
    path::Path,
    sync::Arc,
};
use xxhash_rust::xxh3::Xxh3;

use super::manifest::{lower_hex, AssetEntry};

#[derive(Clone)]
pub struct ObjectStore {
    root: Arc<Dir>,
    max_bytes: u64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct ObjectRecord {
    pub length: u64,
    dev: u64,
    ino: u64,
    mtime: (i64, i64),
    ctime: (i64, i64),
}

impl ObjectRecord {
    pub(super) fn stat(m: &Metadata) -> Self {
        Self {
            length: m.len(),
            dev: m.dev(),
            ino: m.ino(),
            mtime: (m.mtime(), m.mtime_nsec()),
            ctime: (m.ctime(), m.ctime_nsec()),
        }
    }
}

impl ObjectStore {
    pub fn new(root: &Path, max_bytes: u64) -> Result<Self> {
        let root = std::path::absolute(root)?;
        let parent = root.parent().context("object store needs a parent")?;
        // Reject symlinked components before creating anything; Quma's selected
        // data directory is trusted to remain stable during initialization.
        let mut probe = std::path::PathBuf::new();
        for component in parent.components() {
            probe.push(component);
            match fs::symlink_metadata(&probe) {
                Ok(m) => ensure!(m.is_dir(), "aliased or invalid object store parent"),
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                    fs::DirBuilder::new().mode(0o700).create(&probe)?
                }
                Err(e) => return Err(e.into()),
            }
        }
        match fs::DirBuilder::new().mode(0o700).create(&root) {
            Ok(()) => {}
            Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => {}
            Err(e) => return Err(e.into()),
        }
        let m = fs::symlink_metadata(&root)?;
        // SAFETY: geteuid has no preconditions.
        let uid = unsafe { libc::geteuid() };
        ensure!(
            m.is_dir() && m.uid() == uid && m.mode() & 0o777 == 0o700,
            "object store must be a private directory owned by Quma"
        );
        let dir = Dir::open_ambient_dir(&root, cap_std::ambient_authority())?;
        let opened = dir.open(".")?.into_std().metadata()?;
        ensure!(
            opened.dev() == m.dev() && opened.ino() == m.ino(),
            "object store directory was replaced"
        );
        Ok(Self {
            root: Arc::new(dir),
            max_bytes,
        })
    }

    fn open(&self, digest: &str) -> Result<File> {
        use cap_std::fs::OpenOptionsExt;
        ensure!(lower_hex(digest, 32), "invalid object digest");
        let mut opts = OpenOptions::new();
        opts.read(true)
            .custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK);
        let file = self.root.open_with(digest, &opts)?.into_std();
        let meta = file.metadata()?;
        // SAFETY: geteuid has no preconditions.
        let uid = unsafe { libc::geteuid() };
        ensure!(
            meta.is_file() && meta.nlink() == 1 && meta.uid() == uid && meta.mode() & 0o377 == 0,
            "object is not a private, immutable regular file"
        );
        Ok(file)
    }

    pub(super) fn lease(&self, digest: &str, expected: &ObjectRecord) -> Result<File> {
        let file = self.open(digest)?;
        ensure!(
            &ObjectRecord::stat(&file.metadata()?) == expected,
            "verified object changed on disk"
        );
        Ok(file)
    }

    pub(super) fn verify(&self, asset: &AssetEntry) -> Result<ObjectRecord> {
        let mut file = self.open(&asset.xxh3_128)?;
        let before = ObjectRecord::stat(&file.metadata()?);
        ensure!(before.length == asset.length, "object length mismatch");
        ensure!(
            hash_exact(&mut file, asset.length, std::io::sink())? == asset.xxh3_128
                && ObjectRecord::stat(&file.metadata()?) == before,
            "retained object failed verification"
        );
        Ok(before)
    }

    fn lock_directory(&self) -> Result<File> {
        let dir = self.root.open(".")?.into_std();
        // SAFETY: the lock is tied to this live, independently opened directory FD.
        if unsafe { libc::flock(dir.as_raw_fd(), libc::LOCK_EX) } != 0 {
            return Err(std::io::Error::last_os_error().into());
        }
        Ok(dir)
    }

    /// Remove only an object whose verified open handle is still held by this process.
    pub(super) fn retire(&self, digest: &str, expected: &ObjectRecord, held: &File) -> Result<()> {
        let _lock = self.lock_directory()?;
        let current = self.open(digest)?;
        ensure!(
            &ObjectRecord::stat(&held.metadata()?) == expected,
            "retired object handle changed"
        );
        ensure!(
            &ObjectRecord::stat(&current.metadata()?) == expected,
            "retired object was replaced"
        );
        self.root.remove_file(digest)?;
        Ok(())
    }

    pub(super) fn candidate(&self) -> Result<Candidate<'_>> {
        let dir = self.lock_directory()?;
        let mut used = 0u64;
        for entry in self.root.entries()? {
            let entry = entry?;
            let m = self.root.symlink_metadata(entry.file_name())?;
            ensure!(m.is_file(), "unexpected non-regular entry in object store");
            used = used
                .checked_add(m.len())
                .context("object store size overflow")?;
        }
        if used > self.max_bytes {
            return Err(BudgetDeficit(used - self.max_bytes).into());
        }
        Ok(Candidate {
            store: self,
            dir,
            used,
            created: Vec::new(),
            objects: BTreeMap::new(),
        })
    }
}

#[derive(Debug)]
pub(super) struct BudgetDeficit(pub u64);

impl std::fmt::Display for BudgetDeficit {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "publication requires {} additional bytes within the object store cap",
            self.0
        )
    }
}

impl std::error::Error for BudgetDeficit {}

/// Only owns digests this attempt created; reused, committed and pinned
/// digests never enter this list. Keep it armed until durable promotion.
pub(super) struct UnpublishedObjects {
    root: Arc<Dir>,
    created: Vec<String>,
}

impl UnpublishedObjects {
    pub(super) fn commit(&mut self) {
        self.created.clear();
    }
}

fn rollback(root: &Dir, created: &[String]) {
    for digest in created {
        if let Err(err) = root.remove_file(digest) {
            tracing::warn!(err = %err, %digest, "failed to remove unpublished Convoy object");
        }
    }
}

impl Drop for UnpublishedObjects {
    fn drop(&mut self) {
        rollback(&self.root, &self.created);
    }
}

pub(super) struct Candidate<'a> {
    store: &'a ObjectStore,
    dir: File,
    used: u64,
    created: Vec<String>,
    objects: BTreeMap<String, ObjectRecord>,
}

impl Candidate<'_> {
    pub(super) fn reuse(&mut self, asset: &AssetEntry, record: &ObjectRecord) -> Result<()> {
        let _ = self.store.lease(&asset.xxh3_128, record)?;
        self.objects.insert(asset.xxh3_128.clone(), record.clone());
        Ok(())
    }

    pub(super) fn stage(
        &mut self,
        path: String,
        source: &mut File,
        tracked: (Option<&str>, Option<i64>),
    ) -> Result<AssetEntry> {
        let stat = source.metadata()?;
        let length = stat.len();
        ensure!(
            stat.is_file() && stat.nlink() == 1 && length <= i64::MAX as u64,
            "unsafe source or invalid source length"
        );
        if let Some(size) = tracked.1 {
            ensure!(
                i64::try_from(length)? == size,
                "source length differs from tracked length"
            );
        }
        let used = self
            .used
            .checked_add(length)
            .filter(|n| *n <= self.store.max_bytes);
        let root = format!("/proc/self/fd/{}", self.dir.as_raw_fd());
        // Copy and hash on the same read if a whole temporary object fits.
        // Otherwise hash without writing, so an already verified digest can
        // still be reused even when the store is exactly full.
        let mut staged = if used.is_some() && length <= free_bytes(&self.dir)? {
            Some(tempfile::NamedTempFile::new_in(&root)?)
        } else {
            None
        };
        let xxh3_128 = if let Some(temp) = &mut staged {
            #[cfg(test)]
            let mut first = true;
            hash_exact_with(source, length, temp.as_file_mut(), || {
                #[cfg(test)]
                if std::mem::take(&mut first) {
                    super::publisher::test_sync::pause(Path::new(&path));
                }
            })?
        } else {
            hash_exact(source, length, std::io::sink())?
        };
        if let Some(expected) = tracked.0 {
            ensure!(
                xxh3_128 == expected,
                "source checksum differs from tracked checksum"
            );
        }
        ensure!(
            ObjectRecord::stat(&source.metadata()?) == ObjectRecord::stat(&stat),
            "source changed while verifying"
        );
        if !self.objects.contains_key(&xxh3_128) {
            let record = match self.store.open(&xxh3_128) {
                Ok(mut existing) => {
                    let before = ObjectRecord::stat(&existing.metadata()?);
                    ensure!(
                        hash_exact(&mut existing, length, std::io::sink())? == xxh3_128
                            && ObjectRecord::stat(&existing.metadata()?) == before,
                        "retained object failed verification"
                    );
                    before
                }
                Err(e)
                    if e.downcast_ref::<std::io::Error>()
                        .is_some_and(|e| e.kind() == std::io::ErrorKind::NotFound) =>
                {
                    let used = used.ok_or_else(|| {
                        BudgetDeficit(
                            self.used
                                .saturating_add(length)
                                .saturating_sub(self.store.max_bytes)
                                .max(1),
                        )
                    })?;
                    let temp = staged.context("insufficient disk space for a Convoy object")?;
                    temp.as_file()
                        .set_permissions(fs::Permissions::from_mode(0o400))?;
                    temp.as_file().sync_all()?;
                    temp.persist_noclobber(Path::new(&root).join(&xxh3_128))
                        .map_err(|e| e.error)?;
                    self.created.push(xxh3_128.clone());
                    self.used = used;
                    ObjectRecord::stat(&self.store.open(&xxh3_128)?.metadata()?)
                }
                Err(e) => return Err(e),
            };
            self.objects.insert(xxh3_128.clone(), record);
        }
        Ok(AssetEntry {
            path,
            length,
            xxh3_128,
        })
    }

    pub(super) fn finish(mut self) -> Result<(BTreeMap<String, ObjectRecord>, UnpublishedObjects)> {
        if !self.created.is_empty() {
            self.dir.sync_all()?;
        }
        let owned = UnpublishedObjects {
            root: self.store.root.clone(),
            created: std::mem::take(&mut self.created),
        };
        Ok((std::mem::take(&mut self.objects), owned))
    }
}

impl Drop for Candidate<'_> {
    fn drop(&mut self) {
        rollback(&self.store.root, &self.created);
    }
}

fn hash_exact(source: &mut File, remaining: u64, dest: impl Write) -> Result<String> {
    hash_exact_with(source, remaining, dest, || {})
}

fn hash_exact_with(
    source: &mut File,
    mut remaining: u64,
    mut dest: impl Write,
    mut after_write: impl FnMut(),
) -> Result<String> {
    let mut xx = Xxh3::new();
    let mut buf = [0u8; 64 * 1024];
    while remaining != 0 {
        let size = remaining.min(buf.len() as u64) as usize;
        let n = source.read(&mut buf[..size])?;
        ensure!(n != 0, "object ended before declared length");
        let bytes = &buf[..n];
        xx.update(bytes);
        dest.write_all(bytes)?;
        after_write();
        remaining -= n as u64;
    }
    ensure!(
        source.read(&mut buf[..1])? == 0,
        "object exceeds declared length"
    );
    Ok(format!("{:032x}", xx.digest128()))
}

fn free_bytes(dir: &File) -> Result<u64> {
    let mut stats = std::mem::MaybeUninit::<libc::statvfs>::uninit();
    // SAFETY: live FD and valid output slot; a successful call initializes it.
    if unsafe { libc::fstatvfs(dir.as_raw_fd(), stats.as_mut_ptr()) } != 0 {
        return Err(std::io::Error::last_os_error().into());
    }
    let stats = unsafe { stats.assume_init() };
    let available = u128::from(stats.f_bavail).saturating_mul(u128::from(stats.f_frsize));
    Ok(u64::try_from(available).unwrap_or(u64::MAX))
}

#[cfg(test)]
mod tests {
    use super::*;

    // Linux task-local syscall accounting avoids interference from parallel tests.
    fn read_chars() -> u64 {
        let stats = fs::read_to_string("/proc/thread-self/io").expect("read fixture bytes");
        stats
            .lines()
            .find_map(|line| line.strip_prefix("rchar: "))
            .expect("fixture operation must succeed")
            .parse()
            .expect("fixture operation must succeed")
    }

    #[test]
    fn cold_stage_reads_source_once() {
        let fixture = tempfile::tempdir().expect("create isolated fixture root");
        let payload = vec![0x61; 1024 * 1024];
        let source = fixture.path().join("source");
        fs::write(&source, &payload).expect("write fixture bytes");
        let store = ObjectStore::new(&fixture.path().join("store"), payload.len() as u64)
            .expect("create fixture");
        let mut candidate = store.candidate().expect("fixture operation must succeed");
        let mut source = File::open(source).expect("open fixture file or database");
        let before = read_chars();
        let asset = candidate
            .stage("fixture".into(), &mut source, (None, None))
            .expect("fixture operation must succeed");
        let chars = read_chars() - before;
        assert_eq!(asset.length, payload.len() as u64);
        // Allow overhead from reading procfs, but not another pass over the file.
        assert!(
            (asset.length..asset.length + 4096).contains(&chars),
            "read {chars} bytes for {} payload bytes",
            asset.length
        );
    }
}
