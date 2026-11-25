use super::*;
const REL: &str = "BepInEx/plugins/Excluded.dll";

fn recreate_upper(f: &Fixture) {
    std::fs::create_dir_all(f.dirs.headless_upper(1)).unwrap();
    std::fs::create_dir_all(f.dirs.headless_work(1)).unwrap();
}

#[test]
fn headless_overlay_cleanup_preserves_data_on_unmount_failure() {
    let f = Fixture::new(true, true);
    let sentinel = f.dirs.headless_upper(1).join("keep.txt");
    std::fs::write(&sentinel, b"preserve on failed unmount").unwrap();
    let guard = f.guard().unwrap();
    let result = destroy_private_overlay_with(
        &f.dirs,
        1,
        &guard,
        || bail!("simulated unmount failure"),
        |_| panic!("must not remove"),
    );
    assert!(result.is_err());
    assert_eq!(
        std::fs::read(sentinel).unwrap(),
        b"preserve on failed unmount"
    );
}

#[test]
fn generation_retirement_keeps_retired_intent_and_restores_only_current_publication() {
    let mut f = Fixture::new(true, true);
    f.apply().unwrap();
    let guard = f.guard().unwrap();
    destroy_private_overlay(&f.dirs, 1, &guard).unwrap();
    destroy_private_overlay(&f.dirs, 1, &guard).unwrap();
    recreate_upper(&f);
    drop(guard);
    let saved = f.policy.clone();
    f.policy.files.clear();
    assert_eq!(
        f.apply().unwrap().changed,
        1,
        "removed/renamed lower path stays hidden"
    );
    assert_eq!(f.visible(), None);
    f.policy = saved;
    f.include_current();
    f.apply().unwrap();
    assert_eq!(f.visible(), Some(b"current-v2".to_vec()));
    assert_eq!(
        std::fs::read(f.dirs.headless_base.join(REL)).unwrap(),
        b"published"
    );
    assert_eq!(
        std::fs::read(
            f.dirs
                .root
                .join(state_path(1))
                .join("quarantine")
                .join(REL)
                .join(hash(b"published"))
        )
        .unwrap(),
        b"published"
    );
}

#[test]
fn failed_and_interrupted_generation_teardown_refuses_preparation_until_retry() {
    for partial in [false, true] {
        let f = Fixture::new(true, true);
        f.apply().unwrap();
        let guard = f.guard().unwrap();
        let root = Scope::open(&f.dirs.root).unwrap();
        let before = load(&root, 1).unwrap();
        let result = destroy_private_overlay_with(
            &f.dirs,
            1,
            &guard,
            || bail!("unmount injected"),
            |_| panic!("must not remove after failed unmount"),
        );
        assert!(result.unwrap_err().to_string().contains("unmount"));
        assert_eq!(
            serde_json::to_vec(&load(&root, 1).unwrap()).unwrap(),
            serde_json::to_vec(&before).unwrap()
        );
        let result = destroy_private_overlay_with(
            &f.dirs,
            1,
            &guard,
            || Ok(()),
            |path| {
                if partial {
                    std::fs::remove_dir_all(path)?;
                }
                bail!("removal/acknowledgement injected")
            },
        );
        assert!(result.is_err());
        let pending = load(&root, 1).unwrap();
        assert!(pending.retiring.is_some());
        assert_eq!(
            pending.entries[Path::new(REL)].mask,
            before.entries[Path::new(REL)].mask
        );
        if partial {
            recreate_upper(&f);
        }
        assert!(reconcile_private_overlay(&f.dirs, &f.policy, 1, &guard).is_err());
        if partial {
            // An unrelated new generation is not a retry of authorized teardown.
            assert!(destroy_private_overlay(&f.dirs, 1, &guard).is_err());
            std::fs::remove_dir_all(f.dirs.headless_overlay(1)).unwrap();
        }
        resume_overlay_retirement(&f.dirs, 1, &guard).unwrap();
        recreate_upper(&f);
        reconcile_private_overlay(&f.dirs, &f.policy, 1, &guard).unwrap();
        assert_eq!(f.visible(), None);
    }
}

#[test]
fn retirement_evidence_failure_preserves_active_authority_and_upper() {
    let f = Fixture::new(true, true);
    f.apply().unwrap();
    let guard = f.guard().unwrap();
    let root = Scope::open(&f.dirs.root).unwrap();
    let before = serde_json::to_vec(&load(&root, 1).unwrap()).unwrap();
    let outside = tempfile::tempdir().unwrap();
    let alias = f.dirs.root.join(state_path(1)).join("retired");
    std::os::unix::fs::symlink(outside.path(), &alias).unwrap();
    assert!(destroy_private_overlay(&f.dirs, 1, &guard).is_err());
    assert_eq!(
        serde_json::to_vec(&load(&root, 1).unwrap()).unwrap(),
        before
    );
    assert!(f.dirs.headless_upper(1).exists());
    assert_eq!(std::fs::read_dir(outside.path()).unwrap().count(), 0);
    std::fs::remove_file(alias).unwrap();
    destroy_private_overlay(&f.dirs, 1, &guard).unwrap();
}

#[test]
fn partially_removed_upper_finishes_only_via_durable_generation_intent() {
    let f = Fixture::new(true, true);
    f.apply().unwrap();
    let guard = f.guard().unwrap();
    let result = destroy_private_overlay_with(
        &f.dirs,
        1,
        &guard,
        || Ok(()),
        |_| {
            std::fs::remove_dir_all(f.dirs.headless_upper(1))?;
            bail!("interrupted after upper removal")
        },
    );
    assert!(result.is_err());
    assert!(f.dirs.headless_overlay(1).exists());
    resume_overlay_retirement(&f.dirs, 1, &guard).unwrap();
    assert!(!f.dirs.headless_overlay(1).exists());
    recreate_upper(&f);
    reconcile_private_overlay(&f.dirs, &f.policy, 1, &guard).unwrap();
}

#[test]
fn new_generation_never_inherits_unowned_native_mask_authority() {
    let mut f = Fixture::new(false, true);
    f.apply().unwrap();
    let guard = f.guard().unwrap();
    destroy_private_overlay(&f.dirs, 1, &guard).unwrap();
    recreate_upper(&f);
    let state = f.dirs.root.join(state_path(1));
    let mut mounted = Mounted::new(&f.dirs, 1, &state).unwrap();
    std::fs::remove_file(mounted.path.join(REL)).unwrap();
    mounted.unmount().unwrap();
    let root = Scope::open(&f.dirs.root).unwrap();
    reconcile_private_overlay(&f.dirs, &f.policy, 1, &guard).unwrap();
    assert_eq!(
        load(&root, 1).unwrap().entries[Path::new(REL)].phase,
        "retired"
    );
    f.include_current();
    assert!(reconcile_private_overlay(&f.dirs, &f.policy, 1, &guard)
        .unwrap_err()
        .to_string()
        .contains("unowned restoration mask"));
}

#[test]
fn legacy_adoption_is_readonly_in_check_and_retains_same_upper_ownership() {
    let f = Fixture::new(false, true);
    f.apply().unwrap();
    let root = Scope::open(&f.dirs.root).unwrap();
    let state = Scope::open(&f.dirs.root.join(state_path(1))).unwrap();
    let mut legacy = load(&root, 1).unwrap();
    legacy.version = 1;
    legacy.generation = None;
    save(&state, &legacy).unwrap();
    let bytes = state.data(Path::new("journal.json")).unwrap().unwrap();
    check_private_overlay(&f.dirs, &f.policy, 1).unwrap();
    assert_eq!(
        state.data(Path::new("journal.json")).unwrap().unwrap(),
        bytes
    );
    crate::overlay::prepare_work_dir(&f.dirs.headless_work(1)).unwrap();
    f.apply().unwrap();
    let adopted = load(&root, 1).unwrap();
    assert_eq!(adopted.version, 2);
    assert_eq!(
        adopted.entries[Path::new(REL)].mask,
        legacy.entries[Path::new(REL)].mask
    );
    assert!(adopted.generation.is_some());
}
// Fixture-created journals/native views must honor the hardened adapter's
// private-state permission contract, just as PolicyOperationGuard does.
fn create_private_state(path: &Path) {
    use std::os::unix::fs::DirBuilderExt;
    std::fs::DirBuilder::new()
        .recursive(true)
        .mode(0o700)
        .create(path)
        .unwrap();
}
struct Fixture {
    _temp: tempfile::TempDir,
    dirs: QumaDirs,
    policy: PolicySnapshot,
}
impl Fixture {
    fn new(in_upper: bool, in_lower: bool) -> Self {
        let temp = tempfile::tempdir().unwrap();
        let dirs = QumaDirs::from_root(temp.path().to_owned());
        for p in [
            dirs.headless_upper(1),
            dirs.headless_work(1),
            dirs.headless_base.clone(),
            dirs.mod_overlay(),
        ] {
            std::fs::create_dir_all(p).unwrap();
        }
        for p in [
            Some(dirs.mod_overlay()),
            in_upper.then(|| dirs.headless_upper(1)),
            in_lower.then(|| dirs.headless_base.clone()),
        ]
        .into_iter()
        .flatten()
        {
            std::fs::create_dir_all(p.join("BepInEx/plugins")).unwrap();
            std::fs::write(p.join(REL), b"published").unwrap();
        }
        let db = crate::db::Database::open_in_memory().unwrap();
        let id = db
            .insert_mod(None, None, "Excluded", None, "1", "local", None, None)
            .unwrap();
        db.save_groups_atomic(&[(
            "No Headless".into(),
            "no-headless".into(),
            "optional".into(),
            true,
            vec![id],
        )])
        .unwrap();
        db.insert_file(id, REL, Some(&hash(b"published")), Some(9))
            .unwrap();
        let policy = PolicySnapshot::load(&db).unwrap();
        Self {
            _temp: temp,
            dirs,
            policy,
        }
    }
    fn apply(&self) -> Result<ReconcileReport> {
        let guard = self.guard()?;
        reconcile_private_overlay(&self.dirs, &self.policy, 1, &guard)
    }
    fn guard(&self) -> Result<PolicyOperationGuard> {
        // Parallel tests spawn native daemons: a fork can briefly retain a lock
        // descriptor until exec closes CLOEXEC fds. No fixture shares its root.
        let mut attempts = 0;
        let guard = loop {
            match PolicyOperationGuard::acquire(&self.dirs, 1) {
                Ok(g) => break g,
                Err(e) if attempts < 100 && e.to_string().contains("busy") => {
                    attempts += 1;
                    std::thread::sleep(std::time::Duration::from_millis(2));
                }
                Err(e) => return Err(e),
            }
        };
        Ok(guard)
    }
    fn visible(&self) -> Option<Vec<u8>> {
        let state = self.dirs.root.join(state_path(1));
        create_private_state(&state);
        let mut m = Mounted::new(&self.dirs, 1, &state).unwrap();
        let b = Scope::open(&m.path).unwrap().data(Path::new(REL)).unwrap();
        m.unmount().unwrap();
        b
    }
    fn include_current(&mut self) {
        std::fs::write(self.dirs.mod_overlay().join(REL), b"current-v2").unwrap();
        let p = self.policy.files.get_mut(Path::new(REL)).unwrap();
        p.excluded = false;
        p.hash = Some(hash(b"current-v2"));
        p.size = Some(10);
    }
}
#[test]
fn existing_upper_exclusion_removes_observed_payload() {
    let mut f = Fixture::new(true, false);
    assert_eq!(f.visible(), Some(b"published".to_vec()));
    assert_eq!(f.apply().unwrap().changed, 1);
    assert_eq!(f.visible(), None);
    let backup = f
        .dirs
        .root
        .join(state_path(1))
        .join("quarantine")
        .join(REL)
        .join(hash(b"published"));
    assert_eq!(std::fs::read(&backup).unwrap(), b"published");
    assert_eq!(f.apply().unwrap().changed, 0);
    f.include_current();
    f.apply().unwrap();
    assert_eq!(f.visible(), Some(b"current-v2".to_vec()));
    assert_eq!(std::fs::read(backup).unwrap(), b"published");
    assert_eq!(f.apply().unwrap().changed, 0);
}
#[test]
fn lower_mask_retirement_and_current_reinclusion() {
    let mut f = Fixture::new(false, true);
    f.apply().unwrap();
    assert_eq!(f.visible(), None);
    assert_eq!(
        std::fs::read(f.dirs.headless_base.join(REL)).unwrap(),
        b"published"
    );
    let saved = f.policy.clone();
    f.policy.files.clear();
    f.apply().unwrap();
    assert_eq!(f.visible(), None);
    f.policy = saved;
    f.include_current();
    f.apply().unwrap();
    assert_eq!(f.visible(), Some(b"current-v2".to_vec()));
    assert_eq!(
        std::fs::read(f.dirs.headless_base.join(REL)).unwrap(),
        b"published"
    );
}
#[test]
fn modified_late_target_refuses_before_mutation() {
    let mut f = Fixture::new(true, false);
    let later = PathBuf::from("BepInEx/plugins/ZModified.dll");
    f.policy
        .files
        .insert(later.clone(), f.policy.files[Path::new(REL)].clone());
    std::fs::write(f.dirs.headless_upper(1).join(&later), b"custom").unwrap();
    assert!(f.apply().is_err());
    assert_eq!(
        std::fs::read(f.dirs.headless_upper(1).join(REL)).unwrap(),
        b"published"
    );
}
#[test]
fn readonly_absent_and_symlink_refusal() {
    let f = Fixture::new(false, false);
    assert_eq!(
        check_private_overlay(&f.dirs, &f.policy, 1)
            .unwrap()
            .changed,
        0
    );
    assert!(!f.dirs.root.join("headless-policy").exists());
    std::os::unix::fs::symlink(
        f.dirs.mod_overlay().join("BepInEx"),
        f.dirs.headless_upper(1).join("BepInEx"),
    )
    .unwrap();
    assert!(f.apply().is_err());
}
#[test]
fn unowned_native_mask_is_preserved() {
    let f = Fixture::new(false, true);
    let state = f.dirs.root.join("probe");
    std::fs::create_dir(&state).unwrap();
    let mut m = Mounted::new(&f.dirs, 1, &state).unwrap();
    std::fs::remove_file(m.path.join(REL)).unwrap();
    m.unmount().unwrap();
    let before = Scope::open(&f.dirs.headless_upper(1))
        .unwrap()
        .identity(Path::new(REL))
        .unwrap();
    assert_eq!(f.apply().unwrap().changed, 0);
    assert_eq!(f.visible(), None);
    assert_eq!(
        Scope::open(&f.dirs.headless_upper(1))
            .unwrap()
            .identity(Path::new(REL))
            .unwrap(),
        before
    );
}
#[test]
fn mounted_overlay_refuses() {
    let f = Fixture::new(true, false);
    let state = f.dirs.root.join("probe");
    std::fs::create_dir(&state).unwrap();
    let mut m = Mounted::new(&f.dirs, 1, &state).unwrap();
    assert!(f.apply().is_err());
    m.unmount().unwrap();
}
#[test]
fn interrupted_hide_journal_recovers() {
    let f = Fixture::new(false, true);
    f.apply().unwrap();
    let state = Scope::open(&f.dirs.root.join(state_path(1))).unwrap();
    let root = Scope::open(&f.dirs.root).unwrap();
    let mut j = load(&root, 1).unwrap();
    let e = j.entries.get_mut(Path::new(REL)).unwrap();
    e.phase = "hide-intent".into();
    e.mask = None;
    save(&state, &j).unwrap();
    f.apply().unwrap();
    assert_eq!(f.visible(), None);
    let mut included = f;
    included.include_current();
    included.apply().unwrap();
    assert_eq!(included.visible(), Some(b"current-v2".to_vec()));
}

#[test]
fn interrupted_before_unlink_retries_and_after_restore_finalizes() {
    let mut f = Fixture::new(false, true);
    let state_abs = f.dirs.root.join(state_path(1));
    create_private_state(&state_abs);
    let state = Scope::open(&state_abs).unwrap();
    let mut j = Journal {
        version: 1,
        ..Journal::default()
    };
    j.entries.insert(
        PathBuf::from(REL),
        Entry {
            phase: "hide-intent".into(),
            original_hash: Some(hash(b"published")),
            mask: None,
            restored_hash: None,
            previous_hash: None,
        },
    );
    save(&state, &j).unwrap();
    f.apply().unwrap();
    assert_eq!(f.visible(), None);
    f.include_current();
    f.apply().unwrap();
    let root = Scope::open(&f.dirs.root).unwrap();
    let mut j = load(&root, 1).unwrap();
    j.entries.get_mut(Path::new(REL)).unwrap().phase = "restore-intent".into();
    save(&state, &j).unwrap();
    f.apply().unwrap();
    assert_eq!(f.visible(), Some(b"current-v2".to_vec()));
    assert_eq!(
        load(&root, 1).unwrap().entries[Path::new(REL)].phase,
        "included"
    );
}
#[test]
fn interrupted_partial_restore_temp_is_recoverable() {
    let mut f = Fixture::new(false, true);
    f.apply().unwrap();
    f.include_current();
    let root = Scope::open(&f.dirs.root).unwrap();
    let state = Scope::open(&f.dirs.root.join(state_path(1))).unwrap();
    let mut j = load(&root, 1).unwrap();
    let e = j.entries.get_mut(Path::new(REL)).unwrap();
    e.phase = "restore-intent".into();
    e.restored_hash = Some(hash(b"current-v2"));
    save(&state, &j).unwrap();
    std::fs::write(
        f.dirs.headless_upper(1).join(restore_temp(Path::new(REL))),
        b"part",
    )
    .unwrap();
    f.apply().unwrap();
    assert_eq!(f.visible(), Some(b"current-v2".to_vec()));
}
#[test]
fn included_update_and_retired_restoration_use_owned_bytes_only() {
    let mut f = Fixture::new(true, false);
    f.apply().unwrap();
    f.include_current();
    f.apply().unwrap();
    std::fs::write(f.dirs.mod_overlay().join(REL), b"third").unwrap();
    let p = f.policy.files.get_mut(Path::new(REL)).unwrap();
    p.hash = Some(hash(b"third"));
    p.size = Some(5);
    // Simulate interruption after a newer restore intent, before rename.
    let root = Scope::open(&f.dirs.root).unwrap();
    let state = Scope::open(&f.dirs.root.join(state_path(1))).unwrap();
    let mut j = load(&root, 1).unwrap();
    let e = j.entries.get_mut(Path::new(REL)).unwrap();
    e.phase = "restore-intent".into();
    e.previous_hash = Some(hash(b"current-v2"));
    e.restored_hash = Some(hash(b"third"));
    save(&state, &j).unwrap();
    f.apply().unwrap();
    assert_eq!(f.visible(), Some(b"third".to_vec()));
    f.policy.files.clear();
    f.apply().unwrap();
    assert_eq!(f.visible(), None);
}
#[test]
fn lower_added_after_upper_deletion_cannot_resurface_retired_bytes() {
    let mut f = Fixture::new(true, false);
    f.apply().unwrap();
    f.policy.files.clear();
    std::fs::create_dir_all(f.dirs.headless_base.join("BepInEx/plugins")).unwrap();
    std::fs::write(f.dirs.headless_base.join(REL), b"published").unwrap();
    f.apply().unwrap();
    assert_eq!(f.visible(), None);
}
#[test]
fn configs_wine_profiles_sources_and_addons_preserved() {
    let mut f = Fixture::new(true, true);
    for rel in [
        "BepInEx/config/custom.cfg",
        "wine/drive_c/user.dat",
        "profiles/identity.json",
        "BepInEx/plugins/Control.dll",
    ] {
        let p = f.dirs.headless_upper(1).join(rel);
        std::fs::create_dir_all(p.parent().unwrap()).unwrap();
        std::fs::write(p, b"custom-control").unwrap();
    }
    let addon = "BepInEx/plugins/Addon.dll";
    std::fs::write(f.dirs.headless_upper(1).join(addon), b"published").unwrap();
    std::fs::write(f.dirs.mod_overlay().join(addon), b"published").unwrap();
    f.policy
        .files
        .insert(PathBuf::from(addon), f.policy.files[Path::new(REL)].clone());
    assert_eq!(f.apply().unwrap().changed, 2);
    assert_eq!(
        std::fs::read(f.dirs.mod_overlay().join(REL)).unwrap(),
        b"published"
    );
    assert_eq!(
        std::fs::read(f.dirs.headless_base.join(REL)).unwrap(),
        b"published"
    );
    for rel in [
        "BepInEx/config/custom.cfg",
        "wine/drive_c/user.dat",
        "profiles/identity.json",
        "BepInEx/plugins/Control.dll",
    ] {
        assert_eq!(
            std::fs::read(f.dirs.headless_upper(1).join(rel)).unwrap(),
            b"custom-control"
        );
    }
}
#[test]
fn unsafe_paths_guards_and_owned_custom_data_refuse() {
    let mut f = Fixture::new(true, false);
    let mut unsafe_policy = f.policy.clone();
    unsafe_policy.files.insert(
        PathBuf::from("../escape"),
        f.policy.files[Path::new(REL)].clone(),
    );
    assert!(check_private_overlay(&f.dirs, &unsafe_policy, 1).is_err());
    let other = PolicyOperationGuard::acquire(&f.dirs, 2).unwrap();
    assert!(reconcile_private_overlay(&f.dirs, &f.policy, 1, &other).is_err());
    drop(other);
    f.apply().unwrap();
    f.include_current();
    f.apply().unwrap();
    std::fs::write(f.dirs.headless_upper(1).join(REL), b"user-custom").unwrap();
    assert!(f.apply().is_err());
    assert_eq!(
        std::fs::read(f.dirs.headless_upper(1).join(REL)).unwrap(),
        b"user-custom"
    );
}
// I1 regressions: physical upper absence is not merged destination absence.
fn add_lower(f: &Fixture, bytes: &[u8]) {
    let path = f.dirs.headless_base.join(REL);
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(path, bytes).unwrap();
}
#[test]
fn i1_upper_deletion_then_lower_v1_and_current_v2_reinclusion() {
    i1_upper_deletion_then_lower_reinclusion(b"published");
}
#[test]
fn i1_upper_deletion_then_lower_current_and_current_v2_reinclusion() {
    i1_upper_deletion_then_lower_reinclusion(b"current-v2");
}
fn i1_upper_deletion_then_lower_reinclusion(lower: &[u8]) {
    let mut f = Fixture::new(true, false);
    f.apply().unwrap();
    assert_eq!(f.visible(), None);
    add_lower(&f, lower);
    f.include_current();
    assert_eq!(f.visible(), Some(lower.to_vec()));
    assert_eq!(f.apply().unwrap().changed, 1);
    assert_eq!(f.visible(), Some(b"current-v2".to_vec()));
    assert_eq!(
        std::fs::read(f.dirs.headless_base.join(REL)).unwrap(),
        lower
    );
    assert_eq!(f.apply().unwrap().changed, 0);
}
fn persist_pending_hide(f: &Fixture) {
    let state_abs = f.dirs.root.join(state_path(1));
    create_private_state(&state_abs);
    let state = Scope::open(&state_abs).unwrap();
    let mut j = Journal {
        version: 1,
        ..Journal::default()
    };
    j.entries.insert(
        PathBuf::from(REL),
        Entry {
            phase: "hide-intent".into(),
            original_hash: Some(hash(b"published")),
            mask: None,
            restored_hash: None,
            previous_hash: None,
        },
    );
    save(&state, &j).unwrap();
}
#[test]
fn i1_pending_lower_hide_reversed_directly_to_current_included() {
    let mut f = Fixture::new(false, true);
    persist_pending_hide(&f);
    f.include_current();
    assert_eq!(f.visible(), Some(b"published".to_vec()));
    assert_eq!(f.apply().unwrap().changed, 1);
    assert_eq!(f.visible(), Some(b"current-v2".to_vec()));
    assert_eq!(
        std::fs::read(f.dirs.headless_base.join(REL)).unwrap(),
        b"published"
    );
    assert_eq!(f.apply().unwrap().changed, 0);
}
#[test]
fn i1_unknown_visible_lower_is_refused_in_complete_plan() {
    let mut f = Fixture::new(true, false);
    f.apply().unwrap();
    add_lower(&f, b"custom-lower");
    f.include_current();
    // This earlier excluded path would be destructively removed if planning
    // were deferred until each action. The unsafe restore must block it first.
    let earlier = PathBuf::from("BepInEx/plugins/AAA.dll");
    let mut p = f.policy.files[Path::new(REL)].clone();
    p.excluded = true;
    f.policy.files.insert(earlier.clone(), p);
    std::fs::write(f.dirs.headless_upper(1).join(&earlier), b"current-v2").unwrap();
    std::fs::write(f.dirs.mod_overlay().join(&earlier), b"current-v2").unwrap();
    assert!(check_private_overlay(&f.dirs, &f.policy, 1)
        .unwrap_err()
        .to_string()
        .contains("unverifiable restoration lower"));
    assert!(f
        .apply()
        .unwrap_err()
        .to_string()
        .contains("unverifiable restoration lower"));
    assert_eq!(f.visible(), Some(b"custom-lower".to_vec()));
    assert_eq!(
        std::fs::read(f.dirs.headless_upper(1).join(earlier)).unwrap(),
        b"current-v2"
    );
    assert_eq!(
        std::fs::read(f.dirs.headless_base.join(REL)).unwrap(),
        b"custom-lower"
    );
}
#[test]
fn i1_pending_hide_does_not_authorize_custom_upper() {
    let mut f = Fixture::new(false, true);
    persist_pending_hide(&f);
    f.include_current();
    let upper = f.dirs.headless_upper(1).join(REL);
    std::fs::create_dir_all(upper.parent().unwrap()).unwrap();
    std::fs::write(&upper, b"custom-upper").unwrap();
    assert!(check_private_overlay(&f.dirs, &f.policy, 1)
        .unwrap_err()
        .to_string()
        .contains("owned payload modified"));
    assert!(f.apply().is_err());
    assert_eq!(f.visible(), Some(b"custom-upper".to_vec()));
    assert_eq!(std::fs::read(upper).unwrap(), b"custom-upper");
}
#[test]
fn i1_unsupported_visible_lower_refuses_without_mutation() {
    let mut f = Fixture::new(true, false);
    f.apply().unwrap();
    f.include_current();
    let lower = f.dirs.headless_base.join(REL);
    std::fs::create_dir_all(lower.parent().unwrap()).unwrap();
    let name = std::ffi::CString::new(lower.as_os_str().as_encoded_bytes()).unwrap();
    assert_eq!(unsafe { libc::mkfifo(name.as_ptr(), 0o600) }, 0);
    assert!(check_private_overlay(&f.dirs, &f.policy, 1).is_err());
    assert!(f.apply().is_err());
    use std::os::unix::fs::FileTypeExt;
    assert!(std::fs::symlink_metadata(lower)
        .unwrap()
        .file_type()
        .is_fifo());
    assert!(!f.dirs.headless_upper(1).join(REL).exists());
}
#[test]
fn i1_later_unowned_native_mask_is_not_consumed() {
    let mut f = Fixture::new(false, true);
    f.apply().unwrap();
    f.include_current();
    f.apply().unwrap();
    let state = f.dirs.root.join(state_path(1));
    let mut m = Mounted::new(&f.dirs, 1, &state).unwrap();
    std::fs::remove_file(m.path.join(REL)).unwrap();
    m.unmount().unwrap();
    let upper = Scope::open(&f.dirs.headless_upper(1)).unwrap();
    let id = upper.identity(Path::new(REL)).unwrap();
    assert!(id.is_some());
    assert!(f
        .apply()
        .unwrap_err()
        .to_string()
        .contains("unowned restoration mask"));
    assert_eq!(upper.identity(Path::new(REL)).unwrap(), id);
    assert_eq!(f.visible(), None);
}
#[test]
fn i1_native_opaque_ancestor_remains_opaque_during_restore() {
    let mut f = Fixture::new(false, true);
    persist_pending_hide(&f);
    let control = Path::new("BepInEx/plugins/UnownedControl.dll");
    std::fs::write(f.dirs.headless_base.join(control), b"lower-control").unwrap();
    let state = f.dirs.root.join(state_path(1));
    let mut m = Mounted::new(&f.dirs, 1, &state).unwrap();
    std::fs::remove_file(m.path.join(REL)).unwrap();
    std::fs::remove_file(m.path.join(control)).unwrap();
    std::fs::remove_dir(m.path.join("BepInEx/plugins")).unwrap();
    std::fs::create_dir(m.path.join("BepInEx/plugins")).unwrap();
    m.unmount().unwrap();
    assert!(Scope::open(&f.dirs.headless_upper(1))
        .unwrap()
        .scoped
        .blocks_lower_layer(control)
        .unwrap());
    f.include_current();
    assert_eq!(f.apply().unwrap().changed, 1);
    assert_eq!(f.visible(), Some(b"current-v2".to_vec()));
    let mut m = Mounted::new(&f.dirs, 1, &state).unwrap();
    assert_eq!(Scope::open(&m.path).unwrap().data(control).unwrap(), None);
    m.unmount().unwrap();
    assert!(Scope::open(&f.dirs.headless_upper(1))
        .unwrap()
        .scoped
        .blocks_lower_layer(control)
        .unwrap());
    assert_eq!(
        std::fs::read(f.dirs.headless_base.join(control)).unwrap(),
        b"lower-control"
    );
    assert_eq!(
        std::fs::read(f.dirs.headless_base.join(REL)).unwrap(),
        b"published"
    );
    assert_eq!(f.apply().unwrap().changed, 0);
}
#[test]
fn corrupted_journal_and_quarantine_aliases_fail_closed() {
    let f = Fixture::new(true, false);
    let state_abs = f.dirs.root.join(state_path(1));
    create_private_state(&state_abs);
    std::fs::write(state_abs.join("journal.json"), b"not-json").unwrap();
    assert!(f.apply().is_err());
    std::fs::remove_file(state_abs.join("journal.json")).unwrap();
    std::os::unix::fs::symlink(f.dirs.mod_overlay(), state_abs.join("quarantine")).unwrap();
    assert!(check_private_overlay(&f.dirs, &f.policy, 1).is_err());
    assert!(f.apply().is_err());
    assert_eq!(
        std::fs::read(f.dirs.headless_upper(1).join(REL)).unwrap(),
        b"published"
    );
}
