use std::fs::File;
use std::path::Path;

use anyhow::{bail, Result};

use crate::dirs::QumaDirs;

use super::scoped::ScopedDir;

/// Scoped reads from the installation's overlay view, without mounting it.
pub(crate) struct LayeredFiles {
    scopes: Vec<(usize, ScopedDir)>,
}

impl LayeredFiles {
    pub(crate) fn open(dirs: &QumaDirs) -> Result<Self> {
        let roots = if dirs.is_legacy() {
            vec![dirs.spt_server.clone()]
        } else {
            vec![
                dirs.runtime_upper(),
                dirs.mod_overlay(),
                dirs.spt_server.clone(),
            ]
        };
        let mut scopes = Vec::new();
        for (layer, root) in roots.into_iter().enumerate() {
            match ScopedDir::open(&root) {
                Ok(scope) => scopes.push((layer, scope)),
                Err(e) if is_not_found(&e) => continue,
                Err(e) => return Err(e),
            }
        }
        Ok(Self { scopes })
    }

    pub(crate) fn read(&self, relative: &Path, client_only: bool) -> Result<File> {
        self.read_inner(relative, client_only, None)?
            .map(|(_, file)| file)
            .ok_or_else(|| anyhow::anyhow!("file does not exist in any installation layer"))
    }

    #[allow(dead_code)] // Convoy v2 mounts only in the ephemeral service for now.
    pub(crate) fn read_filtered(
        &self,
        relative: &Path,
        accept: &dyn Fn(&Path) -> bool,
    ) -> Result<File> {
        self.read_inner(relative, true, Some(accept))?
            .map(|(_, file)| file)
            .ok_or_else(|| anyhow::anyhow!("file does not exist in any installation layer"))
    }

    /// Returns the exact logical layer and file, or absence from all layers.
    pub(crate) fn read_exact_source(
        &self,
        relative: &Path,
        client_only: bool,
    ) -> Result<Option<(usize, File)>> {
        self.read_inner(relative, client_only, Some(&|opened| opened == relative))
    }

    fn read_inner(
        &self,
        relative: &Path,
        client_only: bool,
        accept: Option<&dyn Fn(&Path) -> bool>,
    ) -> Result<Option<(usize, File)>> {
        for (layer, scope) in &self.scopes {
            let result = if client_only {
                scope.read_client_overlay_file(relative)
            } else {
                scope.read_overlay_file(relative)
            };
            match result {
                Ok(Some(file)) => {
                    if let Some(accept) = accept {
                        if !accept(&scope.opened_relative_path(&file)?) {
                            bail!("opened file is outside the allowed namespace");
                        }
                    }
                    return Ok(Some((*layer, file)));
                }
                Ok(None) => bail!("file is deleted in an installation layer"),
                Err(e) if is_not_found(&e) && !scope.blocks_lower_layer(relative)? => continue,
                Err(e) => return Err(e),
            }
        }
        Ok(None)
    }
}

fn is_not_found(error: &anyhow::Error) -> bool {
    error
        .downcast_ref::<std::io::Error>()
        .is_some_and(|e| e.kind() == std::io::ErrorKind::NotFound)
}
