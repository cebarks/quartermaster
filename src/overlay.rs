use std::path::{Path, PathBuf};

use anyhow::{bail, Context, Result};

#[derive(Debug, Clone)]
pub struct OverlayMount {
    /// Lower directories, bottom-most first. Reversed for the lowerdir= argument
    /// (fuse-overlayfs wants highest-priority first).
    pub lower_dirs: Vec<PathBuf>,
    /// Writable upper layer. All writes land here.
    pub upper_dir: PathBuf,
    /// OverlayFS internal work directory. Emptied before each mount.
    pub work_dir: PathBuf,
    /// Mount point where the merged view appears.
    pub merged_dir: PathBuf,
}

/// Reinitialize an unused work directory for either a host or Podman overlay.
/// Call only before mounting, never while the overlay is in use.
pub(crate) fn prepare_work_dir(work_dir: &Path) -> Result<()> {
    if work_dir.exists() {
        // Kernel OverlayFS leaves an empty mode-000 work/work directory that
        // recursive removal cannot read rootlessly. Only descend through a real
        // outer directory; remove_dir_all below unlinks outer symlinks safely.
        let metadata = std::fs::symlink_metadata(work_dir)
            .context("failed to inspect overlay work directory")?;
        if metadata.file_type().is_dir() {
            let _ = std::fs::remove_dir(work_dir.join("work"));
        }
        std::fs::remove_dir_all(work_dir)
            .with_context(|| format!("failed to clean work dir {}", work_dir.display()))?;
    }
    std::fs::create_dir_all(work_dir)
        .with_context(|| format!("failed to create work dir {}", work_dir.display()))?;
    Ok(())
}

/// Remove an overlay root only after its container has been removed/unmounted.
pub(crate) fn remove_unused_overlay_dir(overlay_dir: &Path) -> Result<()> {
    if std::fs::symlink_metadata(overlay_dir)?.file_type().is_dir() {
        prepare_work_dir(&overlay_dir.join("work"))?;
    }
    std::fs::remove_dir_all(overlay_dir)
        .with_context(|| format!("failed to remove overlay dir {}", overlay_dir.display()))
}

impl OverlayMount {
    pub fn lowerdir_arg(&self) -> String {
        self.lower_dirs
            .iter()
            .rev()
            .map(|p| p.display().to_string())
            .collect::<Vec<_>>()
            .join(":")
    }

    pub fn mount_opts(&self) -> String {
        let lowerdir = self.lowerdir_arg();
        // SAFETY: getuid/getgid are always safe — no preconditions, no failure mode.
        let uid = unsafe { libc::getuid() };
        let gid = unsafe { libc::getgid() };
        format!(
            "lowerdir={},upperdir={},workdir={},squash_to_uid={},squash_to_gid={}",
            lowerdir,
            self.upper_dir.display(),
            self.work_dir.display(),
            uid,
            gid,
        )
    }

    pub fn prepare_dirs(&self) -> Result<()> {
        std::fs::create_dir_all(&self.upper_dir)
            .with_context(|| format!("failed to create upper dir {}", self.upper_dir.display()))?;

        prepare_work_dir(&self.work_dir)?;

        std::fs::create_dir_all(&self.merged_dir).with_context(|| {
            format!("failed to create merged dir {}", self.merged_dir.display())
        })?;

        Ok(())
    }

    pub fn is_mounted(&self) -> Result<bool> {
        let mounts =
            std::fs::read_to_string("/proc/mounts").context("failed to read /proc/mounts")?;
        let target = self.merged_dir.display().to_string();
        Ok(mounts.lines().any(|line| {
            let parts: Vec<&str> = line.split_whitespace().collect();
            parts.len() >= 3 && parts[1] == target && parts[2] == "fuse.fuse-overlayfs"
        }))
    }

    pub fn mount(&self) -> Result<()> {
        if self.is_mounted()? {
            tracing::debug!(
                merged = %self.merged_dir.display(),
                "overlay already mounted, skipping"
            );
            return Ok(());
        }

        self.prepare_dirs()?;

        let opts = self.mount_opts();

        let status = std::process::Command::new("fuse-overlayfs")
            .arg("-o")
            .arg(&opts)
            .arg(self.merged_dir.as_os_str())
            .status()
            .context("failed to execute fuse-overlayfs — is it installed?")?;

        if !status.success() {
            bail!(
                "fuse-overlayfs failed with exit code {:?} for {}",
                status.code(),
                self.merged_dir.display()
            );
        }

        tracing::info!(
            merged = %self.merged_dir.display(),
            "overlay mounted"
        );
        Ok(())
    }

    pub fn unmount(&self) -> Result<()> {
        if !self.is_mounted()? {
            tracing::debug!(
                merged = %self.merged_dir.display(),
                "overlay not mounted, skipping unmount"
            );
            return Ok(());
        }

        let status = std::process::Command::new("fusermount3")
            .arg("-u")
            .arg(self.merged_dir.as_os_str())
            .status()
            .context("failed to execute fusermount3 — is it installed?")?;

        if !status.success() {
            bail!(
                "fusermount3 failed with exit code {:?} for {}",
                status.code(),
                self.merged_dir.display()
            );
        }

        tracing::info!(
            merged = %self.merged_dir.display(),
            "overlay unmounted"
        );
        Ok(())
    }

    #[allow(dead_code)]
    pub fn ensure_mounted(&self) -> Result<()> {
        if !self.is_mounted()? {
            self.mount()?;
        }
        Ok(())
    }
}

pub fn check_prerequisites() -> Result<()> {
    for binary in &["fuse-overlayfs", "fusermount3"] {
        let result = std::process::Command::new("which")
            .arg(binary)
            .output()
            .with_context(|| format!("failed to check for {binary}"))?;

        if !result.status.success() {
            bail!("{binary} is not installed. Install it with: dnf install fuse-overlayfs fuse3");
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn prepare_dirs_cleans_inaccessible_empty_kernel_work_directory() {
        use std::os::unix::fs::PermissionsExt;
        let tmp = tempfile::tempdir().unwrap();
        let mount = OverlayMount {
            lower_dirs: vec![tmp.path().join("base")],
            upper_dir: tmp.path().join("upper"),
            work_dir: tmp.path().join("work"),
            merged_dir: tmp.path().join("merged"),
        };
        let internal = mount.work_dir.join("work");
        std::fs::create_dir_all(&internal).unwrap();
        std::fs::set_permissions(&internal, std::fs::Permissions::from_mode(0)).unwrap();
        let result = mount.prepare_dirs();
        if internal.exists() {
            std::fs::set_permissions(&internal, std::fs::Permissions::from_mode(0o700)).unwrap();
        }
        assert!(result.is_ok(), "{result:?}");
        assert!(mount.work_dir.is_dir());
        assert!(!internal.exists());
    }

    #[test]
    fn prepare_dirs_does_not_follow_internal_work_symlink() {
        use std::os::unix::fs::symlink;
        let tmp = tempfile::tempdir().unwrap();
        let mount = OverlayMount {
            lower_dirs: vec![tmp.path().join("base")],
            upper_dir: tmp.path().join("upper"),
            work_dir: tmp.path().join("work"),
            merged_dir: tmp.path().join("merged"),
        };
        let outside = tmp.path().join("outside");
        std::fs::create_dir_all(&outside).unwrap();
        std::fs::write(outside.join("sentinel"), b"preserve").unwrap();
        std::fs::create_dir_all(&mount.work_dir).unwrap();
        symlink(&outside, mount.work_dir.join("work")).unwrap();
        mount.prepare_dirs().unwrap();
        assert_eq!(
            std::fs::read(outside.join("sentinel")).unwrap(),
            b"preserve"
        );
    }

    #[test]
    fn prepare_dirs_does_not_follow_outer_work_symlink() {
        use std::os::unix::fs::symlink;
        let tmp = tempfile::tempdir().unwrap();
        let mount = OverlayMount {
            lower_dirs: vec![tmp.path().join("base")],
            upper_dir: tmp.path().join("upper"),
            work_dir: tmp.path().join("work"),
            merged_dir: tmp.path().join("merged"),
        };
        let outside = tmp.path().join("outside");
        std::fs::create_dir_all(outside.join("work")).unwrap();
        symlink(&outside, &mount.work_dir).unwrap();
        mount.prepare_dirs().unwrap();
        assert!(outside.join("work").is_dir());
        assert!(!std::fs::symlink_metadata(&mount.work_dir)
            .unwrap()
            .file_type()
            .is_symlink());
    }

    #[test]
    fn headless_overlay_removal_cleans_inaccessible_kernel_work() {
        use std::os::unix::fs::PermissionsExt;
        let tmp = tempfile::tempdir().unwrap();
        let overlay = tmp.path().join("client-1");
        let internal = overlay.join("work/work");
        std::fs::create_dir_all(&internal).unwrap();
        std::fs::set_permissions(&internal, std::fs::Permissions::from_mode(0)).unwrap();
        let result = remove_unused_overlay_dir(&overlay);
        if internal.exists() {
            std::fs::set_permissions(&internal, std::fs::Permissions::from_mode(0o700)).unwrap();
        }
        assert!(result.is_ok(), "{result:?}");
        assert!(!overlay.exists());
    }

    #[test]
    fn headless_overlay_removal_never_follows_outer_or_work_symlinks() {
        use std::os::unix::fs::symlink;
        for outer_link in [true, false] {
            let tmp = tempfile::tempdir().unwrap();
            let overlay = tmp.path().join("client-1");
            let outside = tmp.path().join("outside");
            std::fs::create_dir_all(outside.join("work/work")).unwrap();
            if outer_link {
                symlink(&outside, &overlay).unwrap();
            } else {
                std::fs::create_dir_all(&overlay).unwrap();
                symlink(&outside, overlay.join("work")).unwrap();
            }
            remove_unused_overlay_dir(&overlay).unwrap();
            assert!(outside.join("work/work").is_dir());
            assert!(!overlay.exists());
        }
    }

    #[test]
    fn lowerdir_arg_reverses_order() {
        let mount = OverlayMount {
            lower_dirs: vec![PathBuf::from("/base"), PathBuf::from("/mods")],
            upper_dir: PathBuf::from("/upper"),
            work_dir: PathBuf::from("/work"),
            merged_dir: PathBuf::from("/merged"),
        };
        let arg = mount.lowerdir_arg();
        assert_eq!(arg, "/mods:/base");
    }

    #[test]
    fn lowerdir_arg_single() {
        let mount = OverlayMount {
            lower_dirs: vec![PathBuf::from("/base")],
            upper_dir: PathBuf::from("/upper"),
            work_dir: PathBuf::from("/work"),
            merged_dir: PathBuf::from("/merged"),
        };
        assert_eq!(mount.lowerdir_arg(), "/base");
    }

    #[test]
    fn lowerdir_arg_three_layers() {
        let mount = OverlayMount {
            lower_dirs: vec![
                PathBuf::from("/base"),
                PathBuf::from("/mods"),
                PathBuf::from("/patches"),
            ],
            upper_dir: PathBuf::from("/upper"),
            work_dir: PathBuf::from("/work"),
            merged_dir: PathBuf::from("/merged"),
        };
        assert_eq!(mount.lowerdir_arg(), "/patches:/mods:/base");
    }

    #[test]
    fn prepare_dirs_creates_structure() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let mount = OverlayMount {
            lower_dirs: vec![tmp.path().join("lower")],
            upper_dir: tmp.path().join("upper"),
            work_dir: tmp.path().join("work"),
            merged_dir: tmp.path().join("merged"),
        };
        mount.prepare_dirs().expect("prepare_dirs");
        assert!(mount.upper_dir.exists());
        assert!(mount.work_dir.exists());
        assert!(mount.merged_dir.exists());
    }

    #[test]
    fn prepare_dirs_cleans_workdir() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let work = tmp.path().join("work");
        std::fs::create_dir_all(&work).expect("mkdir");
        std::fs::write(work.join("stale"), "junk").expect("write");

        let mount = OverlayMount {
            lower_dirs: vec![tmp.path().join("lower")],
            upper_dir: tmp.path().join("upper"),
            work_dir: work.clone(),
            merged_dir: tmp.path().join("merged"),
        };
        mount.prepare_dirs().expect("prepare_dirs");
        assert!(work.exists());
        assert!(!work.join("stale").exists());
    }

    #[test]
    fn mount_opts_include_squash_to_uid_gid() {
        let mount = OverlayMount {
            lower_dirs: vec![PathBuf::from("/base")],
            upper_dir: PathBuf::from("/upper"),
            work_dir: PathBuf::from("/work"),
            merged_dir: PathBuf::from("/merged"),
        };
        let opts = mount.mount_opts();
        assert!(
            opts.contains("squash_to_uid="),
            "mount options should include squash_to_uid, got: {opts}"
        );
        assert!(
            opts.contains("squash_to_gid="),
            "mount options should include squash_to_gid, got: {opts}"
        );
    }
}
