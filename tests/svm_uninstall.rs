use std::fs;

use spt_quartermaster::{
    config::Config, db::Database, dirs::QumaDirs, ops::remove_mod_by_id, overlay::OverlayMount,
    svm::SvmManager,
};

struct Mounted(OverlayMount);

impl Drop for Mounted {
    fn drop(&mut self) {
        self.0.unmount().expect("unmount test overlay");
    }
}

#[test]
#[ignore = "requires a usable fuse-overlayfs mount and fusermount3"]
fn uninstall_svm_after_mounted_edit_preserves_settings_without_detecting_installation() {
    let tmp = tempfile::tempdir().unwrap();
    let dirs = QumaDirs::from_root(tmp.path().to_path_buf());
    let db = Database::open_in_memory().unwrap();
    let relative = "SPT_Runtime/user/mods/SVM";
    let package = dirs.mod_overlay().join(relative);
    fs::create_dir_all(package.join("Loader")).unwrap();
    fs::create_dir_all(&dirs.spt_server).unwrap();
    let loader = br#"{"CurrentlySelectedPreset":"null"}"#;
    fs::write(package.join("Loader/loader.json"), loader).unwrap();
    fs::write(package.join("ServerValueModifier.dll"), b"SVM payload").unwrap();
    let id = db
        .insert_mod(Some(236), None, "SVM", None, "1.0", "forge", None, None)
        .unwrap();
    for file in ["Loader/loader.json", "ServerValueModifier.dll"] {
        db.insert_file(id, &format!("{relative}/{file}"), None, None)
            .unwrap();
    }

    dirs.spt_overlay().mount().unwrap();
    let mounted = Mounted(dirs.spt_overlay());
    let mut manager = SvmManager::detect_dirs(&dirs).expect("installed SVM detected");
    manager.create_preset("Custom").unwrap();
    manager.set_active_preset("Custom").unwrap();
    let upper = dirs.runtime_upper().join(relative);
    let saved_loader = fs::read(upper.join("Loader/loader.json")).unwrap();
    let saved_preset = fs::read(upper.join("Presets/Custom.json")).unwrap();
    assert_eq!(
        fs::read(package.join("Loader/loader.json")).unwrap(),
        loader
    );
    assert_eq!(manager.active_preset_name(), "Custom");
    drop(manager);
    drop(mounted);

    remove_mod_by_id(&db, &dirs, &Config::default(), id).unwrap();
    assert!(db.get_mod(id).unwrap().is_none());
    assert!(!package.join("ServerValueModifier.dll").exists());
    assert_eq!(
        fs::read(upper.join("Loader/loader.json")).unwrap(),
        saved_loader
    );
    assert_eq!(
        fs::read(upper.join("Presets/Custom.json")).unwrap(),
        saved_preset
    );
    assert!(
        SvmManager::detect_dirs(&dirs).is_none(),
        "preserved settings are not an installed package"
    );

    dirs.spt_overlay().mount().unwrap();
    let _mounted = Mounted(dirs.spt_overlay());
    assert!(
        SvmManager::detect_dirs(&dirs).is_none(),
        "remounting must not resurrect SVM"
    );
}
