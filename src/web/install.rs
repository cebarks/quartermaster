use std::path::Path;
use std::sync::Arc;

use parking_lot::Mutex;

use crate::db::Database;
use crate::forge::client::ForgeClient;
use crate::forge::models::ForgeVersion;

pub async fn web_download_extract_and_record(
    forge: &ForgeClient,
    db: &Arc<Mutex<Database>>,
    spt_dir: &Path,
    mod_id: i64,
    mod_name: &str,
    mod_slug: Option<&str>,
    version: &ForgeVersion,
) -> anyhow::Result<i64> {
    let link = version
        .link
        .as_deref()
        .ok_or_else(|| anyhow::anyhow!("version has no download link"))?;
    let tmp_dir = tempfile::tempdir()?;
    let archive_path = tmp_dir.path().join("mod.zip");
    forge.download_file(link, &archive_path).await?;

    let spt_dir_clone = spt_dir.to_path_buf();
    let extracted =
        actix_web::web::block(move || crate::spt::mods::extract_mod(&archive_path, &spt_dir_clone))
            .await??;

    let version_id = version.id;
    let version_str = version.version.clone();
    let mod_name_owned = mod_name.to_string();
    let mod_slug_owned = mod_slug.map(|s| s.to_string());
    let spt_dir_owned = spt_dir.to_path_buf();
    let db_clone = db.clone();
    let db_for_scan = db.clone();
    let db_id = actix_web::web::block(move || {
        let db = db_clone.lock();
        let tx = db.begin_transaction()?;
        let db_id = db.insert_mod(
            mod_id,
            version_id,
            &mod_name_owned,
            mod_slug_owned.as_deref(),
            &version_str,
        )?;
        for file in &extracted {
            db.insert_file(db_id, &file.path, Some(&file.hash), Some(file.size as i64))?;
        }
        tx.commit()?;
        Ok::<_, anyhow::Error>(db_id)
    })
    .await??;

    let _ = actix_web::web::block(move || {
        crate::ops::scan_and_record_runtime_files(&db_for_scan, db_id, &spt_dir_owned)
    })
    .await;

    Ok(db_id)
}
