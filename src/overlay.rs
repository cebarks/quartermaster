use std::path::PathBuf;

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

        if self.work_dir.exists() {
            std::fs::remove_dir_all(&self.work_dir)
                .with_context(|| format!("failed to clean work dir {}", self.work_dir.display()))?;
        }
        std::fs::create_dir_all(&self.work_dir)
            .with_context(|| format!("failed to create work dir {}", self.work_dir.display()))?;

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
