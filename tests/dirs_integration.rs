use std::path::PathBuf;

use spt_quartermaster::dirs::QumaDirs;

#[test]
fn new_layout_round_trip() {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path();

    let dirs = QumaDirs::from_root(root.to_path_buf());
    std::fs::create_dir_all(dirs.spt_server.join("SPT/SPT_Data/configs")).unwrap();
    std::fs::create_dir_all(dirs.spt_server.join("SPT/user/mods")).unwrap();
    std::fs::create_dir_all(dirs.spt_server.join("BepInEx/plugins")).unwrap();
    std::fs::write(dirs.spt_server.join("SPT/SPT.Server.exe"), "").unwrap();
    std::fs::write(dirs.spt_server.join("SPT/SPT_Data/configs/core.json"), "{}").unwrap();
    std::fs::write(dirs.config_path(), "").unwrap();

    let detected = QumaDirs::detect(Some(root), None).unwrap();
    assert!(!detected.is_legacy());
    assert_eq!(detected.root, root.to_path_buf());
    assert_eq!(detected.spt_server, root.join("spt-server"));
}

#[test]
fn legacy_layout_detected() {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path();

    std::fs::create_dir_all(root.join("SPT/SPT_Data/configs")).unwrap();
    std::fs::create_dir_all(root.join("SPT/user/mods")).unwrap();
    std::fs::create_dir_all(root.join("BepInEx/plugins")).unwrap();
    std::fs::write(root.join("SPT/SPT.Server.exe"), "").unwrap();
    std::fs::write(root.join("SPT/SPT_Data/configs/core.json"), "{}").unwrap();

    let detected = QumaDirs::detect(Some(root), None).unwrap();
    assert!(detected.is_legacy());
    assert_eq!(detected.spt_server, root.to_path_buf());
}

#[test]
fn legacy_paths_match_old_behavior() {
    let dirs = QumaDirs::from_legacy(PathBuf::from("/old/spt"));

    assert_eq!(dirs.db_path(), PathBuf::from("/old/spt/quartermaster.db"));
    assert_eq!(
        dirs.staging_dir(),
        PathBuf::from("/old/spt/quartermaster/.staging")
    );
    assert_eq!(
        dirs.disabled_dir(),
        PathBuf::from("/old/spt/quartermaster/disabled")
    );
    assert_eq!(
        dirs.queue_dir(),
        PathBuf::from("/old/spt/.quartermaster/queued")
    );
    assert_eq!(
        dirs.cache_dir(),
        PathBuf::from("/old/spt/quartermaster-cache")
    );
    assert_eq!(
        dirs.server_mods_dir(),
        PathBuf::from("/old/spt/SPT/user/mods")
    );
}
