use std::{
    fs::File,
    os::unix::fs::MetadataExt,
    path::{Path, PathBuf},
};

use anyhow::{ensure, Context, Result};

use super::{LayeredFiles, ObjectRecord, QumaDirs};

#[derive(Default)]
pub(in crate::convoy) struct Witnesses(Vec<Witness>);

struct Witness {
    path: PathBuf,
    client: bool,
    expected: Option<(usize, ObjectRecord)>,
}

fn identity(file: &File) -> Result<ObjectRecord> {
    let metadata = file.metadata()?;
    ensure!(
        metadata.is_file() && metadata.nlink() == 1,
        "unsafe publication source"
    );
    Ok(ObjectRecord::stat(&metadata))
}

impl Witnesses {
    /// Read exactly the selected scoped file. Record its layer and identity on
    /// both sides of that same read, or record absence from the scoped view.
    pub(in crate::convoy) fn observe<T>(
        &mut self,
        layers: &LayeredFiles,
        path: &Path,
        client: bool,
        read: impl FnOnce(&mut File) -> Result<T>,
    ) -> Result<Option<T>> {
        let (expected, value) =
            if let Some((layer, mut file)) = layers.read_exact_source(path, client)? {
                let before = identity(&file)?;
                let value = read(&mut file)?;
                ensure!(
                    identity(&file)? == before,
                    "publication source changed during read: {}",
                    path.display()
                );
                (Some((layer, before)), Some(value))
            } else {
                (None, None)
            };
        self.0.push(Witness {
            path: path.to_owned(),
            client,
            expected,
        });
        Ok(value)
    }

    pub(super) fn validate(&self, dirs: &QumaDirs) -> Result<()> {
        if self.0.is_empty() {
            return Ok(());
        }
        // Reopen the layer roots too: a newly created or replaced upper layer
        // can change which file the exact same logical path selects.
        let layers = LayeredFiles::open(dirs)?;
        for witness in &self.0 {
            let actual = layers
                .read_exact_source(&witness.path, witness.client)
                .with_context(|| {
                    format!(
                        "cannot re-resolve publication source {}",
                        witness.path.display()
                    )
                })?
                .map(|(layer, file)| -> Result<_> { Ok((layer, identity(&file)?)) })
                .transpose()?;
            ensure!(
                actual == witness.expected,
                "publication source changed before promotion: {}",
                witness.path.display()
            );
        }
        Ok(())
    }
}
