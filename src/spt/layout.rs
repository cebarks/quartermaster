use std::path::{Path, PathBuf};

use anyhow::Result;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RuntimeLayout {
    Current,
    Legacy,
}

impl RuntimeLayout {
    pub fn directory(self) -> &'static str {
        match self {
            Self::Current => "SPT_Runtime",
            Self::Legacy => "SPT",
        }
    }

    pub fn mod_path(self, path: &str) -> String {
        match server_mod_suffix(path) {
            Some(suffix) => format!("{}/user/mods/{suffix}", self.directory()),
            None => path.to_string(),
        }
    }
}

fn has_markers(root: &Path, layout: RuntimeLayout) -> bool {
    let runtime = root.join(layout.directory());
    runtime.join("SPT.Server.deps.json").is_file()
        && runtime.join("SPT_Data/configs/core.json").is_file()
}

/// Detect the server's inner layout, independently of Quartermaster's data layout.
pub fn detect_layout(root: &Path) -> Result<RuntimeLayout> {
    match (
        has_markers(root, RuntimeLayout::Current),
        has_markers(root, RuntimeLayout::Legacy),
    ) {
        (true, true) => anyhow::bail!(
            "ambiguous SPT installation: both SPT_Runtime and SPT contain server markers in {}",
            root.display()
        ),
        (true, false) => Ok(RuntimeLayout::Current),
        (false, true) => Ok(RuntimeLayout::Legacy),
        (false, false) => Ok(
            if root.join("SPT").is_dir() && !root.join("SPT_Runtime").is_dir() {
                RuntimeLayout::Legacy
            } else {
                RuntimeLayout::Current
            },
        ),
    }
}

/// Resolve the runtime directory without guessing on ambiguous installations.
pub fn runtime_dir(root: &Path) -> Result<PathBuf> {
    Ok(root.join(detect_layout(root)?.directory()))
}

pub fn server_mod_suffix(path: &str) -> Option<&str> {
    path.strip_prefix("SPT_Runtime/user/mods/")
        .or_else(|| path.strip_prefix("SPT/user/mods/"))
}

pub fn is_server_path(path: &str) -> bool {
    path.starts_with("SPT_Runtime/") || path.starts_with("SPT/")
}
