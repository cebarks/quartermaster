use std::fs::File;
use std::path::Path;

use anyhow::{bail, Result};

use crate::dirs::QumaDirs;

use super::scoped::ScopedDir;

/// Scoped reads from the installation's overlay view, without mounting it.
pub(crate) struct LayeredFiles {
    scopes: Vec<ScopedDir>,
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
        for root in roots {
            match ScopedDir::open(&root) {
                Ok(scope) => scopes.push(scope),
                Err(e) if is_not_found(&e) => continue,
                Err(e) => return Err(e),
            }
        }
        Ok(Self { scopes })
    }

    pub(crate) fn read(&self, relative: &Path, client_only: bool) -> Result<File> {
        for scope in &self.scopes {
            let result = if client_only {
                scope.read_client_overlay_file(relative)
            } else {
                scope.read_overlay_file(relative)
            };
            match result {
                Ok(Some(file)) => return Ok(file),
                Ok(None) => bail!("file is deleted in an installation layer"),
                Err(e) if is_not_found(&e) && !scope.blocks_lower_layer(relative)? => continue,
                Err(e) => return Err(e),
            }
        }
        bail!("file does not exist in any installation layer")
    }
}

fn is_not_found(error: &anyhow::Error) -> bool {
    error
        .downcast_ref::<std::io::Error>()
        .is_some_and(|e| e.kind() == std::io::ErrorKind::NotFound)
}
