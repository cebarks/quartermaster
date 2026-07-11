use std::sync::Arc;

use parking_lot::Mutex;

use crate::db::Database;
use crate::dirs::QumaDirs;
use crate::forge::client::ForgeClient;
use crate::forge::models::ForgeVersion;

#[allow(clippy::too_many_arguments)]
pub async fn web_download_extract_and_record(
    forge: &ForgeClient,
    db: &Arc<Mutex<Database>>,
    dirs: &QumaDirs,
    config: &crate::config::Config,
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

    let version_id = version.id;
    let version_str = version.version.clone();
    let mod_name_owned = mod_name.to_string();
    let mod_slug_owned = mod_slug.map(|s| s.to_string());
    let db_clone = db.clone();
    let dirs = dirs.clone();
    let config = config.clone();
    let db_id = actix_web::web::block(move || {
        let db = db_clone.lock();
        crate::ops::install_mod_from_archive(&crate::ops::InstallRequest {
            db: &db,
            dirs: &dirs,
            config: &config,
            forge_mod_id: Some(mod_id),
            version_id: Some(version_id),
            name: &mod_name_owned,
            slug: mod_slug_owned.as_deref(),
            version: &version_str,
            archive_path: &archive_path,
            source: crate::ops::ModSource::Forge,
            source_url: None,
        })
    })
    .await??;

    Ok(db_id)
}

pub async fn web_install_from_url(
    forge: &ForgeClient,
    db: &Arc<Mutex<Database>>,
    dirs: &QumaDirs,
    config: &crate::config::Config,
    url: &str,
    mod_name: &str,
) -> anyhow::Result<i64> {
    let tmp_dir = tempfile::tempdir()?;
    let archive_path = tmp_dir.path().join("mod.zip");
    forge.download_file(url, &archive_path).await?;

    let mod_name_owned = mod_name.to_string();
    let url_owned = url.to_string();
    let db_clone = db.clone();
    let dirs = dirs.clone();
    let config = config.clone();
    let db_id = actix_web::web::block(move || {
        let db = db_clone.lock();
        crate::ops::install_mod_from_archive(&crate::ops::InstallRequest {
            db: &db,
            dirs: &dirs,
            config: &config,
            forge_mod_id: None,
            version_id: None,
            name: &mod_name_owned,
            slug: None,
            version: "unknown",
            archive_path: &archive_path,
            source: crate::ops::ModSource::Url,
            source_url: Some(&url_owned),
        })
    })
    .await??;

    Ok(db_id)
}
