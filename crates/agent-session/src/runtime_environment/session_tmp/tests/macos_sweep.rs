use super::*;
use crate::coordination::broker::{self, test_support};
use pretty_assertions::assert_eq;
use std::os::fd::AsRawFd;
use std::os::unix::fs::OpenOptionsExt;

const BOOT: &str = "11111111-1111-4111-8111-111111111111";
const IDENTITY: &str = crate::DELETE_TMUX_IDENTITY_KEY;

fn fixture_runtime(fixture: &Fixture, group: libc::pid_t) -> (SessionRecord, PathBuf) {
    let context = fixture.context();
    let mut record = fixture.record("stopped-runtime", json!({}));
    record.extra.insert(
        IDENTITY.to_owned(),
        json!({
            "launch_id": "fixture-runtime",
            "macos_boot_id": BOOT,
            "session_id": "$1",
            "pane_id": "%1",
            "pane_pid": group,
            "process_group_id": group,
        }),
    );
    let launch_tmp = prepare_launch(&fixture.state_dir(), &record)
        .unwrap()
        .unwrap();
    crate::write_session_record(&context, &record).unwrap();
    broker::prepare(&context, &record).unwrap();
    broker::provision(&context, &record).unwrap();
    broker::revoke(&context, &record).unwrap();
    mark_stopped(&fixture.state_dir(), &record.id);
    (record, launch_tmp)
}

fn macos_sweep(
    fixture: &Fixture,
    tmux: &Path,
    boot: Option<&str>,
    grace: Duration,
) -> sweep::SweepReport {
    sweep::sweep_with_runtime_check(
        &fixture.context(),
        tmux,
        grace,
        None,
        |context, tmux, record| sweep::stopped_macos(context, tmux, record, boot),
    )
    .unwrap()
}

fn assert_retained(
    fixture: &Fixture,
    launch_tmp: &Path,
    record: &SessionRecord,
    tmux: &Path,
    boot: Option<&str>,
    reason: &'static str,
) {
    let record_path = session_dir(&fixture.context(), &record.id).join("session.json");
    let before = fs::read(&record_path).unwrap();
    let report = macos_sweep(fixture, tmux, boot, Duration::ZERO);
    assert_eq!(report.removed, 0);
    assert_eq!(report.retained, [(reason, 1)].into());
    assert!(fixture.tmp_dir(&record.id).is_dir());
    assert!(fs::symlink_metadata(launch_tmp).is_ok());
    assert_eq!(fs::read(record_path).unwrap(), before);
}

#[test]
fn same_boot_stopped_runtime_reclaims_backing_without_deleting_record() {
    let fixture = Fixture::new();
    let (record, launch_tmp) =
        fixture_runtime(&fixture, test_support::verified_absent_process_group());
    let backing = fixture.tmp_dir(&record.id);
    let record_path = session_dir(&fixture.context(), &record.id).join("session.json");
    let before = fs::read(&record_path).unwrap();
    let tmux = fixture.tmux(false);
    assert_retained(
        &fixture,
        &launch_tmp,
        &record,
        &fixture.tmux(true),
        Some(BOOT),
        "live",
    );
    let report = macos_sweep(&fixture, &tmux, Some(BOOT), Duration::from_secs(3600));
    assert_eq!(report.retained, [("grace", 1)].into());
    assert!(backing.is_dir());
    assert!(fs::symlink_metadata(&launch_tmp).is_ok());

    let report = macos_sweep(&fixture, &tmux, Some(BOOT), Duration::ZERO);
    assert_eq!(
        report.removed,
        1,
        "retained: {:?}, backing present: {}, launch path present: {}",
        report.retained,
        backing.is_dir(),
        fs::symlink_metadata(&launch_tmp).is_ok()
    );
    assert!(!backing.exists());
    assert_eq!(
        fs::symlink_metadata(&launch_tmp).unwrap_err().kind(),
        io::ErrorKind::NotFound
    );
    assert_eq!(fs::read(record_path).unwrap(), before);
}

#[test]
fn same_boot_sweep_retains_unreadable_and_replaced_evidence() {
    let fixture = Fixture::new();
    let context = fixture.context();
    let (record, launch_tmp) =
        fixture_runtime(&fixture, test_support::verified_absent_process_group());
    let tmux = fixture.tmux(false);

    let mut changed = record.clone();
    changed.extra.remove(IDENTITY);
    crate::write_session_record(&context, &changed).unwrap();
    assert_retained(
        &fixture,
        &launch_tmp,
        &changed,
        &tmux,
        Some(BOOT),
        "unknown",
    );

    changed = record.clone();
    changed.extra.get_mut(IDENTITY).unwrap()["launch_id"] = json!("previous-runtime");
    crate::write_session_record(&context, &changed).unwrap();
    assert_retained(
        &fixture,
        &launch_tmp,
        &changed,
        &tmux,
        Some(BOOT),
        "unknown",
    );

    changed = record.clone();
    changed.runtime.as_mut().unwrap().generation += 1;
    crate::write_session_record(&context, &changed).unwrap();
    assert_retained(
        &fixture,
        &launch_tmp,
        &changed,
        &tmux,
        Some(BOOT),
        "unknown",
    );

    crate::write_session_record(&context, &record).unwrap();
    assert_retained(&fixture, &launch_tmp, &record, &tmux, None, "unknown");
    assert_retained(
        &fixture,
        &launch_tmp,
        &record,
        &fixture.dir.path().join("unavailable-tmux"),
        Some(BOOT),
        "unknown",
    );

    fs::write(
        context.state_dir.join("coordination/registry.json"),
        "invalid",
    )
    .unwrap();
    assert_retained(&fixture, &launch_tmp, &record, &tmux, Some(BOOT), "unknown");
}

#[test]
fn same_boot_sweep_retains_a_live_process_group_even_after_broker_stop() {
    let fixture = Fixture::new();
    // SAFETY: getpgrp has no preconditions. The test only uses signal zero.
    let group = unsafe { libc::getpgrp() };
    assert!(group > 1);
    let (record, launch_tmp) = fixture_runtime(&fixture, group);
    let tmux = fixture.tmux(false);
    assert_retained(&fixture, &launch_tmp, &record, &tmux, Some(BOOT), "live");
}

#[test]
fn same_boot_sweep_retains_a_live_heartbeat_writer_and_unverified_beat() {
    let fixture = Fixture::new();
    let context = fixture.context();
    let (record, launch_tmp) =
        fixture_runtime(&fixture, test_support::verified_absent_process_group());
    let tmux = fixture.tmux(false);
    let incarnation = &record.runtime.as_ref().unwrap().launch_id;
    let lock_name = format!(
        "broker-{}.lock",
        crate::coordination::digest_bytes(incarnation.as_bytes())
    );
    let lock = fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o600)
        .open(crate::coordination::coordination_dir(&context, &record.id).join(lock_name))
        .unwrap();
    // SAFETY: lock owns a valid descriptor for this disposable file.
    assert_eq!(
        unsafe { libc::flock(lock.as_raw_fd(), libc::LOCK_EX | libc::LOCK_NB) },
        0
    );
    assert_retained(&fixture, &launch_tmp, &record, &tmux, Some(BOOT), "live");
    drop(lock);

    let heartbeat = crate::coordination::heartbeat_path(&context.state_dir, &record.id);
    write_atomic(&heartbeat, b"unreadable", SECRET_FILE_MODE).unwrap();
    assert_retained(&fixture, &launch_tmp, &record, &tmux, Some(BOOT), "unknown");
    write_atomic(&heartbeat, b"previous-runtime:1\n", SECRET_FILE_MODE).unwrap();
    assert_retained(&fixture, &launch_tmp, &record, &tmux, Some(BOOT), "unknown");
    fs::remove_file(&heartbeat).unwrap();
    assert!(
        ProcessCommand::new("mkfifo")
            .arg(&heartbeat)
            .status()
            .unwrap()
            .success()
    );
    assert_retained(&fixture, &launch_tmp, &record, &tmux, Some(BOOT), "unknown");
}
