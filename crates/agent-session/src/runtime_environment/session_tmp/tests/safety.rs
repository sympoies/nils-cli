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
    ensure_short_path(&dir).unwrap();
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
        let dir = fixture.tmp_dir(id);
        assert!(dir.is_dir());
        assert!(short_root(&dir).unwrap().unwrap().join("t").is_symlink());
    }
    mark_stopped(&fixture.state_dir(), "symlinked");
    let _grace = EnvGuard::set(&fixture._lock, GRACE_ENV, "0");
    assert!(
        prepare_launch(
            &fixture.state_dir(),
            &fixture.record("symlinked", json!({}))
        )
        .is_err()
    );
    let dir = fixture.tmp_dir("symlinked");
    assert!(dir.is_dir());
    assert!(short_root(&dir).unwrap().unwrap().join("t").is_symlink());
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
    assert!(short_root(&dir).unwrap().unwrap().join("t").is_symlink());
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
    let orphan_link = short_root(&orphan_dir).unwrap().unwrap().join("t");
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
    assert!(orphan_link.is_symlink());
    assert!(short_root(&dir).unwrap().unwrap().join("t").is_symlink());
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
    assert!(fs::symlink_metadata(orphan_link).is_err());
    assert!(dir.is_dir());
    assert!(short_root(&dir).unwrap().unwrap().join("t").is_symlink());
}

#[test]
fn short_root_removal_failure_retains_backing_and_is_retryable() {
    let fixture = Fixture::new();
    let record = fixture.persist("partial");
    let link = prepare_launch(&fixture.state_dir(), &record)
        .unwrap()
        .unwrap();
    let root = link.parent().unwrap();
    fs::write(root.join("unexpected"), "canary").unwrap();
    let report = sweep(
        &fixture.context(),
        &fixture.tmux(false),
        Duration::ZERO,
        None,
    )
    .unwrap();
    assert_eq!(report.removed, 0);
    assert_eq!(report.retained.get("remove-failed"), Some(&1));
    assert!(fixture.tmp_dir("partial").is_dir());
    fs::remove_file(root.join("unexpected")).unwrap();
    let report = sweep(
        &fixture.context(),
        &fixture.tmux(false),
        Duration::ZERO,
        None,
    )
    .unwrap();
    assert_eq!(report.removed, 1);
    assert!(!fixture.tmp_dir("partial").exists());
    assert!(!root.exists());
}

#[cfg(target_os = "linux")]
#[test]
fn mounted_backing_retains_short_path_until_unmounted() {
    const INNER: &str = "NILS_TEST_SHORT_MOUNT_INNER";
    if std::env::var_os(INNER).is_some() {
        let fixture = Fixture::new();
        let record = fixture.persist("mounted");
        let link = prepare_launch(&fixture.state_dir(), &record)
            .unwrap()
            .unwrap();
        let backing = fixture.tmp_dir("mounted");
        let source = fixture.dir.path().join("external");
        let mountpoint = backing.join("mount");
        fs::create_dir(&source).unwrap();
        fs::create_dir(&mountpoint).unwrap();
        fs::write(source.join("keep"), "canary").unwrap();
        assert!(
            ProcessCommand::new("mount")
                .arg("--bind")
                .arg(&source)
                .arg(&mountpoint)
                .status()
                .unwrap()
                .success()
        );
        let report = sweep(
            &fixture.context(),
            &fixture.tmux(false),
            Duration::ZERO,
            None,
        )
        .unwrap();
        assert_eq!(report.removed, 0);
        assert_eq!(report.retained.get("untrusted"), Some(&1));
        assert!(backing.is_dir());
        assert!(link.is_symlink());
        assert!(source.join("keep").is_file());
        assert!(
            ProcessCommand::new("umount")
                .arg(&mountpoint)
                .status()
                .unwrap()
                .success()
        );
        let report = sweep(
            &fixture.context(),
            &fixture.tmux(false),
            Duration::ZERO,
            None,
        )
        .unwrap();
        assert_eq!(report.removed, 1);
        assert!(!backing.exists());
        assert!(fs::symlink_metadata(link).is_err());
        return;
    }
    let available = ProcessCommand::new("sudo")
        .args([
            "-n",
            "unshare",
            "--mount",
            "--propagation",
            "private",
            "true",
        ])
        .status();
    if !available.is_ok_and(|status| status.success()) {
        eprintln!("skipped: private privileged mount namespace unavailable");
        return;
    }
    let script = "mount -t tmpfs -o mode=1777 tmpfs /tmp; exec env TMPDIR=/tmp NILS_TEST_SHORT_MOUNT_INNER=1 \"$1\" runtime_environment::session_tmp::tests::safety::mounted_backing_retains_short_path_until_unmounted --exact --nocapture";
    let output = ProcessCommand::new("sudo")
        .args([
            "-n",
            "unshare",
            "--mount",
            "--propagation",
            "private",
            "sh",
            "-eu",
            "-c",
            script,
            "fixture",
        ])
        .arg(std::env::current_exe().unwrap())
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "isolated mount test failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
}

#[test]
fn unknown_runtime_retains_backing_and_short_path() {
    let fixture = Fixture::new();
    let record = fixture.persist("unknown");
    let link = prepare_launch(&fixture.state_dir(), &record)
        .unwrap()
        .unwrap();
    let tmux = fixture.dir.path().join("tmux-unknown");
    fs::write(&tmux, "#!/bin/sh\nexit 2\n").unwrap();
    fs::set_permissions(&tmux, fs::Permissions::from_mode(0o700)).unwrap();
    let report = sweep(&fixture.context(), &tmux, Duration::ZERO, None).unwrap();
    assert_eq!(report.removed, 0);
    assert_eq!(report.retained.get("unknown"), Some(&1));
    assert!(fixture.tmp_dir("unknown").is_dir());
    assert!(link.is_symlink());
}
