use std::fs::File;
use std::path::{Component, Path, PathBuf};

use anyhow::{bail, Context, Result};
use cap_std::fs::{Dir, OpenOptions};

/// An explicitly selected root. All subsequent I/O uses its directory handle.
pub(crate) struct ScopedDir {
    path: PathBuf,
    dir: Dir,
}

pub(crate) fn relative_path(name: &str) -> Result<PathBuf> {
    let name = name.replace('\\', "/");
    if name.contains('\0') || name.as_bytes().get(1) == Some(&b':') {
        bail!("unsafe filesystem path: {name:?}");
    }
    let path = PathBuf::from(name);
    validate_relative(&path)?;
    Ok(path)
}

fn validate_relative(path: &Path) -> Result<()> {
    if path.as_os_str().is_empty()
        || path
            .components()
            .any(|c| !matches!(c, Component::Normal(_)))
    {
        bail!("unsafe relative filesystem path: {}", path.display());
    }
    Ok(())
}

impl ScopedDir {
    pub(crate) fn open(path: &Path) -> Result<Self> {
        let dir = Dir::open_ambient_dir(path, cap_std::ambient_authority())?;
        Self::bind(path, dir)
    }

    fn bind(path: &Path, dir: Dir) -> Result<Self> {
        let scope = Self {
            path: path.canonicalize()?,
            dir,
        };
        scope.verify_identity()?;
        Ok(scope)
    }

    fn verify_identity(&self) -> Result<()> {
        #[cfg(unix)]
        {
            use cap_std::fs::MetadataExt as CapMetadataExt;
            use std::os::unix::fs::MetadataExt as StdMetadataExt;
            let handle = self.dir.dir_metadata()?;
            let named = std::fs::metadata(&self.path)?;
            if handle.dev() != named.dev() || handle.ino() != named.ino() {
                bail!("filesystem scope directory was replaced");
            }
            Ok(())
        }
        #[cfg(not(unix))]
        {
            bail!("filesystem scope identity verification requires Unix");
        }
    }

    /// Resolve existing aliases, including absolute links contained in the root.
    /// This is not the security boundary: cap-std confines the subsequent I/O.
    pub(crate) fn resolve(&self, path: &Path) -> Result<PathBuf> {
        validate_relative(path)?;
        self.verify_identity()?;
        let mut probe = self.path.join(path);
        let mut missing = Vec::new();
        let mut links = 0;
        let canonical = loop {
            match std::fs::symlink_metadata(&probe) {
                Ok(metadata) => match probe.canonicalize() {
                    Ok(canonical) => break canonical,
                    Err(e)
                        if e.kind() == std::io::ErrorKind::NotFound
                            && metadata.file_type().is_symlink() =>
                    {
                        links += 1;
                        if links > 40 {
                            bail!("too many symbolic links");
                        }
                        let target = std::fs::read_link(&probe)?;
                        probe = if target.is_absolute() {
                            target
                        } else {
                            probe
                                .parent()
                                .context("symlink has no parent")?
                                .join(target)
                        };
                    }
                    Err(e) => return Err(e.into()),
                },
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                    missing.push(
                        probe
                            .file_name()
                            .context("missing path has no filename")?
                            .to_owned(),
                    );
                    if !probe.pop() {
                        return Err(e.into());
                    }
                }
                Err(e) => return Err(e.into()),
            }
        };
        self.verify_identity()?;
        let mut relative = canonical
            .strip_prefix(&self.path)
            .context("filesystem path escapes permitted root")?
            .to_path_buf();
        for part in missing.into_iter().rev() {
            relative.push(part);
        }
        Ok(relative)
    }

    pub(crate) fn child(&self, path: &Path) -> Result<Self> {
        let relative = self.resolve(path)?;
        let dir = self.dir.open_dir(&relative)?;
        let child = Self::bind(&self.path.join(relative), dir)?;
        self.verify_identity()?;
        if !child.path.starts_with(&self.path) {
            bail!("filesystem child directory escapes permitted root");
        }
        Ok(child)
    }

    /// After a missing read, distinguish absent entries from overlay shadows.
    /// Inspect each ancestor without following links, using bound directory handles.
    pub(crate) fn blocks_lower_layer(&self, path: &Path) -> Result<bool> {
        validate_relative(path)?;
        self.verify_identity()?;
        let mut dir = self.dir.try_clone()?;
        for component in path.components() {
            if directory_is_opaque(&dir)? {
                return Ok(true);
            }
            let name = component.as_os_str();
            let mut whiteout = std::ffi::OsString::from(".wh.");
            whiteout.push(name);
            match dir.symlink_metadata(&whiteout) {
                Ok(_) => return Ok(true),
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
                Err(e) => return Err(e.into()),
            }
            match dir.symlink_metadata(name) {
                Ok(metadata) if metadata.is_dir() => dir = dir.open_dir(name)?,
                Ok(_) => return Ok(true),
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(false),
                Err(e) => return Err(e.into()),
            }
        }
        Ok(true)
    }

    pub(crate) fn require_unaliased_directories(&self, path: &Path) -> Result<()> {
        validate_relative(path)?;
        self.verify_identity()?;
        let mut directory = self.dir.try_clone()?;
        for component in path.components() {
            let name = component.as_os_str();
            match directory.symlink_metadata(name) {
                Ok(metadata) if metadata.file_type().is_symlink() => {
                    bail!("directory aliases are unsupported for this filesystem view");
                }
                Ok(metadata) if metadata.is_dir() => directory = directory.open_dir(name)?,
                Ok(_) => break,
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => break,
                Err(error) => return Err(error.into()),
            }
        }
        self.verify_identity()
    }

    pub(crate) fn file_names(&self, path: &Path) -> Result<Vec<std::ffi::OsString>> {
        let child = self.child(path)?;
        let names = child
            .dir
            .entries()?
            .map(|entry| Ok(entry?.file_name()))
            .collect::<Result<_>>()?;
        self.verify_identity()?;
        Ok(names)
    }

    pub(crate) fn read_overlay_file(&self, path: &Path) -> Result<Option<File>> {
        #[cfg(target_os = "linux")]
        {
            use cap_std::fs::MetadataExt;
            let metadata = self.dir.symlink_metadata(self.resolve(path)?)?;
            if is_kernel_whiteout(metadata.mode(), metadata.rdev()) {
                return Ok(None);
            }
        }
        let file = self.read_file(path)?;
        #[cfg(target_os = "linux")]
        {
            use std::os::fd::AsRawFd;
            // Native regular-file whiteouts must be empty; restored data is visible.
            if file.metadata()?.len() != 0 {
                return Ok(Some(file));
            }
            for attr in [
                c"trusted.overlay.whiteout",
                c"user.overlay.whiteout",
                c"user.fuseoverlayfs.whiteout",
            ] {
                // SAFETY: live fd/C name; a null zero-length buffer probes attribute presence.
                let size = unsafe {
                    libc::fgetxattr(file.as_raw_fd(), attr.as_ptr(), std::ptr::null_mut(), 0)
                };
                if size >= 0 {
                    return Ok(None);
                }
                let error = std::io::Error::last_os_error();
                if !matches!(
                    error.raw_os_error(),
                    Some(libc::ENODATA | libc::ENOTSUP | libc::EPERM)
                ) {
                    return Err(error.into());
                }
            }
        }
        Ok(Some(file))
    }

    pub(crate) fn read_file(&self, path: &Path) -> Result<File> {
        let relative = self.resolve(path)?;
        let mut options = OpenOptions::new();
        options.read(true);
        #[cfg(unix)]
        {
            use cap_std::fs::OpenOptionsExt;
            options.custom_flags(libc::O_NONBLOCK);
        }
        let file = self.dir.open_with(relative, &options)?;
        if !file.metadata()?.is_file() {
            bail!("source is not a regular file");
        }
        Ok(file.into_std())
    }

    /// Runtime directory names/aliases must remain stable during classification.
    /// Capability confinement does not depend on this namespace prerequisite.
    pub(crate) fn read_client_file(&self, path: &Path) -> Result<File> {
        self.read_client_overlay_file(path)?
            .context("client source is deleted")
    }

    pub(crate) fn read_client_overlay_file(&self, path: &Path) -> Result<Option<File>> {
        #[cfg(target_os = "linux")]
        {
            use std::os::fd::AsRawFd;
            let Some(file) = self.read_overlay_file(path)? else {
                return Ok(None);
            };
            self.verify_identity()?;
            // Classify the opened descriptor, not a pathname checked before opening.
            // The data stream remains on this capability-opened file handle.
            let opened = std::fs::read_link(format!("/proc/self/fd/{}", file.as_raw_fd()))?;
            let relative = opened
                .strip_prefix(&self.path)
                .context("opened client source is outside the bound installation")?;
            for runtime in ["SPT_Runtime", "SPT"] {
                if relative.starts_with(self.resolve(Path::new(runtime))?) {
                    bail!("client source targets a server runtime tree");
                }
            }
            self.verify_identity()?;
            Ok(Some(file))
        }
        #[cfg(not(target_os = "linux"))]
        {
            let _ = path;
            bail!("opened client-source classification requires Linux");
        }
    }

    pub(crate) fn create_dir_all(&self, path: &Path) -> Result<()> {
        self.dir.create_dir_all(self.resolve(path)?)?;
        Ok(())
    }

    pub(crate) fn create_file(&self, path: &Path) -> Result<File> {
        let relative = self.resolve(path)?;
        if let Some(parent) = relative.parent().filter(|p| !p.as_os_str().is_empty()) {
            self.dir.create_dir_all(parent)?;
        }
        let mut options = OpenOptions::new();
        options.write(true).create(true);
        #[cfg(unix)]
        {
            use cap_std::fs::OpenOptionsExt;
            options.custom_flags(libc::O_NONBLOCK);
        }
        let file = self.dir.open_with(relative, &options)?;
        if !file.metadata()?.is_file() {
            bail!("destination is not a regular file");
        }
        file.set_len(0)?;
        Ok(file.into_std())
    }
}

#[cfg(target_os = "linux")]
fn is_kernel_whiteout(mode: u32, device: u64) -> bool {
    mode & libc::S_IFMT == libc::S_IFCHR && device == 0
}

fn directory_is_opaque(dir: &Dir) -> Result<bool> {
    match dir.symlink_metadata(".wh..wh..opq") {
        Ok(_) => return Ok(true),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
        Err(e) => return Err(e.into()),
    }
    #[cfg(target_os = "linux")]
    {
        use std::os::fd::AsRawFd;
        let directory = dir.open(".")?;
        for attr in [
            c"trusted.overlay.opaque",
            c"user.overlay.opaque",
            c"user.fuseoverlayfs.opaque",
        ] {
            let mut value = [0u8; 16];
            // SAFETY: the live directory fd, C string and writable buffer are valid for this call.
            let size = unsafe {
                libc::fgetxattr(
                    directory.as_raw_fd(),
                    attr.as_ptr(),
                    value.as_mut_ptr().cast(),
                    value.len(),
                )
            };
            if size > 0 && value[0] == b'y' {
                return Ok(true);
            }
            if size < 0 {
                let error = std::io::Error::last_os_error();
                if !matches!(
                    error.raw_os_error(),
                    Some(libc::ENODATA | libc::ENOTSUP | libc::EPERM)
                ) {
                    return Err(error.into());
                }
            }
        }
    }
    Ok(false)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[cfg(target_os = "linux")]
    #[test]
    fn native_whiteout_is_only_a_zero_device_character_node() {
        assert!(is_kernel_whiteout(libc::S_IFCHR | 0o600, 0));
        assert!(!is_kernel_whiteout(libc::S_IFREG | 0o600, 0));
        assert!(!is_kernel_whiteout(libc::S_IFCHR | 0o600, 259));
    }

    #[test]
    fn rejects_unsafe_names() {
        for name in [
            "",
            "/absolute",
            "../parent",
            "a/../b",
            "a\\..\\b",
            "C:\\file",
            "\\\\host\\share",
            "nul\0file",
        ] {
            assert!(relative_path(name).is_err(), "{name:?}");
        }
    }

    #[cfg(unix)]
    #[test]
    fn root_substitution_during_binding_is_rejected() {
        let parent = tempfile::tempdir().unwrap();
        let outside = tempfile::tempdir().unwrap();
        let root = parent.path().join("root");
        std::fs::create_dir(&root).unwrap();
        let captured = Dir::open_ambient_dir(&root, cap_std::ambient_authority()).unwrap();
        std::fs::rename(&root, parent.path().join("original")).unwrap();
        std::os::unix::fs::symlink(outside.path(), &root).unwrap();
        assert!(ScopedDir::bind(&root, captured).is_err());
        assert_eq!(std::fs::read_dir(outside.path()).unwrap().count(), 0);
    }

    #[cfg(unix)]
    #[test]
    fn substituted_root_path_is_rejected() {
        let parent = tempfile::tempdir().unwrap();
        let root = parent.path().join("root");
        std::fs::create_dir(&root).unwrap();
        let scope = ScopedDir::open(&root).unwrap();
        std::fs::rename(&root, parent.path().join("original")).unwrap();
        std::fs::create_dir(&root).unwrap();
        std::fs::write(root.join("file"), b"replacement").unwrap();
        assert!(scope.resolve(Path::new("file")).is_err());
    }

    #[cfg(unix)]
    #[test]
    fn changed_alias_cannot_escape_after_resolution() {
        let root = tempfile::tempdir().unwrap();
        let outside = tempfile::tempdir().unwrap();
        std::fs::create_dir(root.path().join("inside")).unwrap();
        std::fs::write(root.path().join("inside/file"), b"allowed").unwrap();
        std::fs::write(outside.path().join("file"), b"secret").unwrap();
        let scope = ScopedDir::open(root.path()).unwrap();
        let relative = scope.resolve(Path::new("inside/file")).unwrap();
        std::fs::remove_file(root.path().join("inside/file")).unwrap();
        std::fs::remove_dir(root.path().join("inside")).unwrap();
        std::os::unix::fs::symlink(outside.path(), root.path().join("inside")).unwrap();
        assert!(scope.dir.open(&relative).is_err());
        let mut options = OpenOptions::new();
        options.write(true).create(true);
        assert!(scope.dir.open_with(&relative, &options).is_err());
        assert_eq!(
            std::fs::read(outside.path().join("file")).unwrap(),
            b"secret"
        );
    }
}
