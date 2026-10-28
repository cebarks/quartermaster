use std::collections::BTreeSet;
use std::ffi::OsString;
use std::fs::File;
use std::path::{Component, Path, PathBuf};

use anyhow::{bail, ensure, Context, Result};

use crate::dirs::QumaDirs;
use crate::spt::scoped::ScopedDir;

pub(crate) struct ProfileReader {
    layers: Vec<ScopedDir>,
    relative: PathBuf,
}

fn is_not_found(error: &anyhow::Error) -> bool {
    error
        .downcast_ref::<std::io::Error>()
        .is_some_and(|error| error.kind() == std::io::ErrorKind::NotFound)
}

impl ProfileReader {
    pub(crate) fn open(dirs: &QumaDirs) -> Result<Self> {
        let relative = dirs
            .server_runtime_dir()?
            .strip_prefix(&dirs.spt_server)?
            .join("user/profiles");
        let roots = if dirs.is_legacy() {
            vec![dirs.spt_server.clone()]
        } else {
            vec![
                dirs.runtime_upper(),
                dirs.mod_overlay(),
                dirs.spt_server.clone(),
            ]
        };
        let mut layers = Vec::new();
        for root in roots {
            match ScopedDir::open(&root) {
                Ok(layer) => {
                    layer
                        .require_unaliased_directories(&relative)
                        .context("unable to open profile directory safely")?;
                    layers.push(layer);
                }
                Err(error) if is_not_found(&error) => {}
                Err(error) => return Err(error),
            }
        }
        Ok(Self { layers, relative })
    }

    /// Candidate names may be masked in an upper layer; always open through `read`.
    pub(crate) fn candidate_names(&self) -> Result<Vec<OsString>> {
        let mut names = BTreeSet::new();
        for layer in &self.layers {
            match layer.file_names(&self.relative) {
                Ok(entries) => names.extend(entries.into_iter().filter(|name| {
                    !name.as_encoded_bytes().starts_with(b".wh.")
                        && Path::new(name)
                            .extension()
                            .is_some_and(|extension| extension == "json")
                })),
                Err(error) if is_not_found(&error) => {}
                Err(error) => return Err(error),
            }
        }
        Ok(names.into_iter().collect())
    }

    pub(crate) fn read(&self, name: &Path) -> Result<Option<File>> {
        validate_profile_name(name)?;
        let relative = self.relative.join(name);
        for layer in &self.layers {
            match layer.read_overlay_file(&relative) {
                Ok(file) => return Ok(file),
                Err(error) if is_not_found(&error) => {
                    if layer.blocks_lower_layer(&relative)? {
                        return Ok(None);
                    }
                }
                Err(error) => return Err(error),
            }
        }
        Ok(None)
    }

    pub(crate) fn read_id(&self, id: &str) -> Result<Option<File>> {
        self.read(Path::new(&format!("{id}.json")))
    }
}

fn validate_profile_name(name: &Path) -> Result<()> {
    if name.components().count() != 1
        || !matches!(name.components().next(), Some(Component::Normal(_)))
        || name.as_os_str().as_encoded_bytes().contains(&b'\\')
        || name.extension().is_none_or(|extension| extension != "json")
    {
        bail!("unsafe profile filename");
    }
    Ok(())
}

pub(crate) struct ProfileWriter {
    scope: ScopedDir,
}

impl ProfileWriter {
    pub(crate) fn open(dirs: &QumaDirs) -> Result<Self> {
        let root = dirs.profile_write_root()?;
        std::fs::create_dir_all(&root)?;
        let layer = ScopedDir::open(&root)?;
        let relative = dirs
            .server_runtime_dir()?
            .strip_prefix(&dirs.spt_server)?
            .join("user/profiles");
        layer.require_unaliased_directories(&relative)?;
        layer.create_dir_all(&relative)?;
        Ok(Self {
            scope: layer.child(&relative)?,
        })
    }

    pub(crate) fn copy_from(&self, name: &Path, source: &mut File) -> Result<u64> {
        validate_profile_name(name)?;
        let metadata = source.metadata()?;
        ensure!(metadata.is_file(), "backup profile is not a regular file");
        let mut target = self.scope.create_file(name)?;
        target.set_permissions(metadata.permissions())?;
        Ok(std::io::copy(source, &mut target)?)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Read;

    fn fixture() -> (tempfile::TempDir, QumaDirs, PathBuf, PathBuf) {
        let tmp = tempfile::tempdir().unwrap();
        let dirs = QumaDirs::from_root(tmp.path().to_path_buf());
        let relative = Path::new("SPT_Runtime/user/profiles");
        let base = dirs.spt_server.join(relative);
        let upper = dirs.runtime_upper().join(relative);
        std::fs::create_dir_all(&base).unwrap();
        std::fs::create_dir_all(&upper).unwrap();
        (tmp, dirs, base, upper)
    }

    #[test]
    fn unmounted_profiles_include_base_and_upper_with_overrides() {
        let (_tmp, dirs, base, upper) = fixture();
        std::fs::write(base.join("base.json"), "base").unwrap();
        std::fs::write(base.join("shared.json"), "old").unwrap();
        std::fs::write(upper.join("shared.json"), "new").unwrap();
        std::fs::write(upper.join("upper.json"), "upper").unwrap();
        let reader = ProfileReader::open(&dirs).unwrap();
        let names: std::collections::BTreeSet<_> =
            reader.candidate_names().unwrap().into_iter().collect();
        assert_eq!(
            names,
            ["base.json", "shared.json", "upper.json"]
                .into_iter()
                .map(OsString::from)
                .collect()
        );
        let mut content = String::new();
        reader
            .read_id("shared")
            .unwrap()
            .unwrap()
            .read_to_string(&mut content)
            .unwrap();
        assert_eq!(content, "new");
    }

    #[test]
    fn profile_directory_aliases_fail_explicitly_in_any_layer() {
        use std::os::unix::fs::symlink;
        for upper_layer in [false, true] {
            for relative in [
                "SPT_Runtime",
                "SPT_Runtime/user",
                "SPT_Runtime/user/profiles",
            ] {
                let (_tmp, dirs, _base, _upper) = fixture();
                let root = if upper_layer {
                    dirs.runtime_upper()
                } else {
                    dirs.spt_server.clone()
                };
                let original = root.join(relative);
                let target = root.join("alias-target");
                std::fs::rename(&original, &target).unwrap();
                symlink(&target, original).unwrap();
                assert!(ProfileReader::open(&dirs).is_err());
            }
        }
    }

    #[test]
    fn scoped_profile_writer_retains_confined_leaf_aliases() {
        use std::os::unix::fs::symlink;
        let (tmp, dirs, _base, upper) = fixture();
        std::fs::write(upper.join("target.json"), "old").unwrap();
        symlink("target.json", upper.join("alias.json")).unwrap();
        let source_path = tmp.path().join("saved.json");
        std::fs::write(&source_path, "saved").unwrap();
        let mut source = File::open(source_path).unwrap();
        ProfileWriter::open(&dirs)
            .unwrap()
            .copy_from(Path::new("alias.json"), &mut source)
            .unwrap();
        assert!(std::fs::symlink_metadata(upper.join("alias.json"))
            .unwrap()
            .file_type()
            .is_symlink());
        assert_eq!(
            std::fs::read_to_string(upper.join("target.json")).unwrap(),
            "saved"
        );
    }

    #[test]
    fn confined_individual_profile_file_alias_remains_readable() {
        use std::os::unix::fs::symlink;
        let (_tmp, dirs, base, _upper) = fixture();
        std::fs::write(base.join("one.json"), "one").unwrap();
        symlink("one.json", base.join("alias.json")).unwrap();
        let reader = ProfileReader::open(&dirs).unwrap();
        assert_eq!(
            std::io::read_to_string(reader.read_id("alias").unwrap().unwrap()).unwrap(),
            "one"
        );
    }

    #[test]
    fn mod_profiles_override_base_but_runtime_overrides_mods() {
        let (_tmp, dirs, base, upper) = fixture();
        let mods = dirs.mod_overlay().join("SPT_Runtime/user/profiles");
        std::fs::create_dir_all(&mods).unwrap();
        std::fs::write(base.join("one.json"), "base").unwrap();
        std::fs::write(mods.join("one.json"), "mod").unwrap();
        let reader = ProfileReader::open(&dirs).unwrap();
        assert_eq!(
            std::io::read_to_string(reader.read_id("one").unwrap().unwrap()).unwrap(),
            "mod"
        );
        std::fs::write(upper.join("one.json"), "runtime").unwrap();
        assert_eq!(
            std::io::read_to_string(reader.read_id("one").unwrap().unwrap()).unwrap(),
            "runtime"
        );
    }

    #[test]
    fn profile_reads_reject_unsafe_names_and_external_symlinks() {
        use std::os::unix::fs::symlink;
        let (tmp, dirs, base, upper) = fixture();
        std::fs::write(base.join("hidden.json"), "base").unwrap();
        std::fs::write(tmp.path().join("outside.json"), "outside").unwrap();
        symlink("missing.json", upper.join("hidden.json")).unwrap();
        symlink(tmp.path().join("outside.json"), upper.join("external.json")).unwrap();
        let reader = ProfileReader::open(&dirs).unwrap();
        assert!(reader.read_id("hidden").unwrap().is_none());
        assert!(reader.read_id("external").is_err());
        for name in [
            "../outside.json",
            "/outside.json",
            "..\\outside.json",
            "directory/one.json",
            "profile.txt",
        ] {
            assert!(reader.read(Path::new(name)).is_err());
        }
    }

    #[test]
    fn replaced_profile_layer_fails_closed() {
        let (_tmp, dirs, _base, upper) = fixture();
        std::fs::write(upper.join("one.json"), "one").unwrap();
        let reader = ProfileReader::open(&dirs).unwrap();
        std::fs::rename(
            dirs.runtime_upper(),
            dirs.runtime_upper().with_file_name("old-upper"),
        )
        .unwrap();
        std::fs::create_dir_all(dirs.runtime_upper()).unwrap();
        assert!(reader.read_id("one").is_err());
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn profile_reads_respect_native_overlay_whiteout_and_opacity_xattrs() {
        use std::os::fd::AsRawFd;
        let (_tmp, dirs, base, upper) = fixture();
        std::fs::write(base.join("hidden.json"), "base").unwrap();
        std::fs::write(base.join("lower.json"), "lower").unwrap();
        let whiteout = File::create(upper.join("hidden.json")).unwrap();
        // SAFETY: live fd/C name and zero-length attribute value are valid.
        assert_eq!(
            unsafe {
                libc::fsetxattr(
                    whiteout.as_raw_fd(),
                    c"user.overlay.whiteout".as_ptr(),
                    std::ptr::null(),
                    0,
                    0,
                )
            },
            0
        );
        let reader = ProfileReader::open(&dirs).unwrap();
        assert!(reader.read_id("hidden").unwrap().is_none());
        std::fs::write(upper.join("hidden.json"), "restored").unwrap();
        assert_eq!(
            std::io::read_to_string(reader.read_id("hidden").unwrap().unwrap()).unwrap(),
            "restored"
        );
        assert!(reader.read_id("lower").unwrap().is_some());
        let directory = File::open(&upper).unwrap();
        // SAFETY: live fd/C name and one readable value byte are valid.
        assert_eq!(
            unsafe {
                libc::fsetxattr(
                    directory.as_raw_fd(),
                    c"user.overlay.opaque".as_ptr(),
                    b"y".as_ptr().cast(),
                    1,
                    0,
                )
            },
            0
        );
        assert!(reader.read_id("lower").unwrap().is_none());
    }

    #[test]
    fn unmounted_profiles_respect_whiteout_and_opaque_directory() {
        let (_tmp, dirs, base, upper) = fixture();
        std::fs::write(base.join("deleted.json"), "old").unwrap();
        std::fs::write(base.join("base.json"), "base").unwrap();
        std::fs::write(upper.join(".wh.deleted.json"), "").unwrap();
        let reader = ProfileReader::open(&dirs).unwrap();
        assert!(reader.read_id("deleted").unwrap().is_none());
        assert!(reader.read_id("base").unwrap().is_some());
        std::fs::write(upper.join(".wh..wh..opq"), "").unwrap();
        assert!(reader.read_id("base").unwrap().is_none());
    }
}
