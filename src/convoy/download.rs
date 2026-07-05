use std::io::{Cursor, Write};
use std::path::Path;
use zip::write::{SimpleFileOptions, ZipWriter};

use crate::db::Database;

pub fn build_download_zip(
    db: &Database,
    spt_dir: &Path,
    forge_ids: &[i64],
) -> anyhow::Result<Vec<u8>> {
    let files = db.get_files_for_forge_ids(forge_ids)?;
    let buf = Cursor::new(Vec::new());
    let mut zip = ZipWriter::new(buf);
    let options = SimpleFileOptions::default().compression_method(zip::CompressionMethod::Deflated);

    let mut file_count = 0;
    for file in &files {
        if !file.file_path.starts_with("BepInEx/") {
            continue;
        }
        let disk_path = spt_dir.join(&file.file_path);
        if !disk_path.exists() {
            tracing::warn!(
                "convoy download: file not found on disk: {}",
                file.file_path
            );
            continue;
        }
        let data = std::fs::read(&disk_path)?;
        zip.start_file(&file.file_path, options)?;
        zip.write_all(&data)?;
        file_count += 1;
    }

    if file_count == 0 {
        anyhow::bail!("no downloadable files found for requested mod IDs");
    }

    let cursor = zip.finish()?;
    Ok(cursor.into_inner())
}
