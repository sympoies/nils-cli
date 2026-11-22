use super::*;
use pretty_assertions::assert_eq;
use std::os::unix::fs::symlink;

fn owned_dir(fixture: &Fixture, id: &str) -> PathBuf {
    let dir = fixture.tmp_dir(id);
    fs::create_dir_all(&dir).unwrap();
    write_owner(
        &dir,
        &Owner {
            schema_version: OWNER_SCHEMA.to_string(),
            session_id: id.to_string(),
            state_dir: display_path(&fixture.state_dir()),
            stopped_at_epoch: None,
        },
    )
    .unwrap();
    dir
}

#[test]
fn sweep_preserves_locked_unreadable_and_symlinked_directories() {
    let fixture = Fixture::new();
    for id in ["busy", "unreadable", "symlinked"] {
        fixture.persist_stopped(id);
        owned_dir(&fixture, id);
    }
    let _held = acquire_session_record_lock(&fixture.context(), "busy").unwrap();
    fs::write(
        session_dir(&fixture.context(), "unreadable").join("session.json"),
        "invalid",
    )
    .unwrap();
    let outside = fixture.dir.path().join("outside");
    fs::create_dir(&outside).unwrap();
    fs::write(outside.join("keep"), "canary").unwrap();
    symlink(&outside, fixture.tmp_dir("symlinked").join("link")).unwrap();

    let report = sweep(
        &fixture.context(),
        &fixture.tmux(false),
        Duration::ZERO,
        None,
    )
    .unwrap();
    assert_eq!(report.removed, 0);
    assert_eq!(report.retained.get("busy"), Some(&1));
    assert_eq!(report.retained.get("unreadable"), Some(&1));
    assert_eq!(report.retained.get("untrusted"), Some(&1));
    assert_eq!(fs::read_to_string(outside.join("keep")).unwrap(), "canary");
    for id in ["busy", "unreadable", "symlinked"] {
        assert!(fixture.tmp_dir(id).is_dir());
    }
}

#[test]
fn sweep_preserves_symlinked_root_and_writable_directory() {
    let fixture = Fixture::new();
    fixture.persist_stopped("foreign-owner");
    let dir = owned_dir(&fixture, "foreign-owner");
    use std::os::unix::fs::PermissionsExt;
    fs::set_permissions(&dir, fs::Permissions::from_mode(0o777)).unwrap();
    let report = sweep(
        &fixture.context(),
        &fixture.tmux(false),
        Duration::ZERO,
        None,
    )
    .unwrap();
    assert_eq!(report.removed, 0);
    assert_eq!(report.retained.get("untrusted"), Some(&1));
    assert!(dir.is_dir());
    let root = dir.parent().unwrap();
    let moved = fixture.dir.path().join("moved-tmp");
    fs::rename(root, &moved).unwrap();
    symlink(&moved, root).unwrap();
    let error = sweep(
        &fixture.context(),
        &fixture.tmux(false),
        Duration::ZERO,
        None,
    )
    .unwrap_err();
    assert_eq!(error.code(), "session-tmpdir-root-untrusted");
    assert!(moved.join("foreign-owner").is_dir());
}

#[test]
fn sweep_keeps_future_stop_and_recent_orphan_until_retention() {
    let fixture = Fixture::new();
    fixture.persist_stopped("future");
    let dir = owned_dir(&fixture, "future");
    let mut owner = read_owner(&dir, "future").unwrap();
    owner.stopped_at_epoch = Some(now_epoch() + 3600);
    write_owner(&dir, &owner).unwrap();
    let orphan_dir = owned_dir(&fixture, "orphan");
    let report = sweep(
        &fixture.context(),
        &fixture.tmux(false),
        Duration::from_secs(60),
        None,
    )
    .unwrap();
    assert_eq!(report.removed, 0);
    assert!(dir.is_dir());
    assert!(orphan_dir.is_dir());
    // Make only the synthetic directory old, rather than sleeping or changing
    // any host clock. An orphan must satisfy the same retention bound.
    fs::File::open(&orphan_dir)
        .unwrap()
        .set_times(fs::FileTimes::new().set_modified(UNIX_EPOCH + Duration::from_secs(1)))
        .unwrap();
    let report = sweep(
        &fixture.context(),
        &fixture.tmux(false),
        Duration::from_secs(60),
        None,
    )
    .unwrap();
    assert_eq!(report.removed, 1);
    assert!(!orphan_dir.exists());
    assert!(dir.is_dir());
}
