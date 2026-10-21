use super::process_supervisor::spawn_authenticated_supervisor_output_fixture;
#[cfg(target_os = "linux")]
use super::process_supervisor::{PublishedPidCleanup, SupervisorFixture};
use super::*;

#[test]
fn authenticated_target_status_one_remains_a_git_domain_result() {
    use std::os::unix::net::UnixStream;

    let (completion_reader, mut completion_writer) =
        UnixStream::pair().expect("target status completion socket");
    write_process_supervisor_completion(
        &mut completion_writer,
        ProcessSupervisorCompletion {
            kind: ProcessSupervisorCompletionKind::Target,
            cleanup_complete: true,
            exit_code: 1,
        },
    )
    .expect("deliver target status completion");
    drop(completion_writer);
    let mut command = Command::new("/bin/sh");
    command
        .args(["-c", "exit 1"])
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .process_group(0);
    let child = command
        .spawn()
        .expect("spawn authenticated status-one fixture");
    let spawned = SpawnedOutputChild {
        child,
        supervised: true,
        cleanup_proof: Some(completion_reader),
        fallback_owner: None,
    };

    let output = collect_output_child_until(spawned, Instant::now() + TEST_WAIT, 1024, 1024, 1024)
        .expect("authenticated target status one remains available to Git consumers");

    assert_eq!(output.status.code(), Some(1));
}

#[test]
#[cfg(any(target_os = "linux", target_os = "macos"))]
fn authenticated_supervisor_producer_preserves_normal_target_status_one() {
    let root = tempfile::TempDir::new().expect("target status producer root");
    let spawned = spawn_authenticated_supervisor_output_fixture(
        "target-status-one",
        &root.path().join("unused.pid"),
    );

    let output = collect_output_child_until(spawned, Instant::now() + TEST_WAIT, 1024, 1024, 1024)
        .expect("authenticated target status one remains a Git-domain result");

    assert_eq!(output.status.code(), Some(exit::RUNTIME));
}

#[test]
#[cfg(any(target_os = "linux", target_os = "macos"))]
fn authenticated_supervisor_producer_internal_failure_is_not_git_status_one() {
    let root = tempfile::TempDir::new().expect("internal failure producer root");
    let spawned = spawn_authenticated_supervisor_output_fixture(
        "internal-failure",
        &root.path().join("unused.pid"),
    );

    let error = collect_output_child_until(spawned, Instant::now() + TEST_WAIT, 1024, 1024, 1024)
        .expect_err("authenticated internal failure must not enter the Git status domain");

    assert_eq!(
        error
            .downcast_ref::<DirtyCheckoutError>()
            .expect("typed authenticated internal failure")
            .kind(),
        DirtyCheckoutErrorKind::ResourceUnavailable
    );
}

#[test]
fn authenticated_supervisor_internal_failure_cannot_become_git_status_one() {
    use std::os::unix::net::UnixStream;

    let (completion_reader, mut completion_writer) =
        UnixStream::pair().expect("internal failure completion socket");
    write_process_supervisor_completion(
        &mut completion_writer,
        ProcessSupervisorCompletion {
            kind: ProcessSupervisorCompletionKind::InternalFailure,
            cleanup_complete: false,
            exit_code: 1,
        },
    )
    .expect("deliver internal failure completion");
    drop(completion_writer);
    let mut command = Command::new("/bin/sh");
    command
        .args(["-c", "exit 1"])
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .process_group(0);
    let child = command.spawn().expect("spawn internal status-one fixture");
    let spawned = SpawnedOutputChild {
        child,
        supervised: true,
        cleanup_proof: Some(completion_reader),
        fallback_owner: None,
    };

    let error = collect_output_child_until(spawned, Instant::now() + TEST_WAIT, 1024, 1024, 1024)
        .expect_err("authenticated internal failure must not become Git status one");

    assert_eq!(
        error
            .downcast_ref::<DirtyCheckoutError>()
            .expect("typed authenticated internal failure")
            .kind(),
        DirtyCheckoutErrorKind::ResourceUnavailable
    );
}

#[test]
fn supervised_child_without_authenticated_completion_cannot_return_git_status_one() {
    use std::os::unix::net::UnixStream;

    let (completion_reader, completion_writer) =
        UnixStream::pair().expect("supervisor completion socket");
    drop(completion_writer);
    let mut command = Command::new("/bin/sh");
    command
        .args(["-c", "exit 1"])
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .process_group(0);
    let child = command
        .spawn()
        .expect("spawn status-one supervisor fixture");
    let spawned = SpawnedOutputChild {
        child,
        supervised: true,
        cleanup_proof: Some(completion_reader),
        fallback_owner: None,
    };

    let error = collect_output_child_until(spawned, Instant::now() + TEST_WAIT, 1024, 1024, 1024)
        .expect_err("missing authenticated completion must not become Git status one");

    assert_eq!(
        error
            .downcast_ref::<DirtyCheckoutError>()
            .expect("typed missing supervisor completion error")
            .kind(),
        DirtyCheckoutErrorKind::ResourceUnavailable
    );
}

// macOS hosted runners can exhaust kqueue tracking while creating this fixture; see #1290.
#[cfg(target_os = "linux")]
#[test]
fn missing_supervisor_completion_cleans_a_prescanned_escaped_descendant() {
    use std::os::unix::net::UnixStream;

    let root = tempfile::TempDir::new().expect("missing completion escaped root");
    let pid_path = root.path().join("escaped.pid");
    let start_path = root.path().join("start");
    let release_path = root.path().join("release");
    let (completion_reader, completion_writer) =
        UnixStream::pair().expect("missing completion socket");
    drop(completion_writer);
    let mut command = Command::new(env::current_exe().expect("current test executable"));
    command
        .arg("output_cleanup_escaped_descendant_test_fixture")
        .arg("--nocapture")
        .env("NILS_OUTPUT_CLEANUP_ESCAPED_DESCENDANT", "1")
        .env("NILS_OUTPUT_CLEANUP_EXIT_AFTER_PUBLISH", "1")
        .env("NILS_OUTPUT_CLEANUP_START_PATH", &start_path)
        .env("NILS_OUTPUT_CLEANUP_RELEASE_PATH", &release_path)
        .env("NILS_PROCESS_SUPERVISOR_TEST_PID_PATH", &pid_path)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .process_group(0);
    let child = command
        .spawn()
        .expect("spawn unexpected supervisor exit fixture");
    let mut fallback_owner = ProcessOwner::for_root(child.id() as libc::pid_t)
        .expect("create scoped escaped fallback owner");
    fs::write(&start_path, []).expect("release escaped fixture start gate");
    let publication_deadline = Instant::now() + TEST_WAIT;
    while !pid_path.exists() {
        assert!(
            Instant::now() < publication_deadline,
            "escaped fixture did not publish its PID"
        );
        std::thread::sleep(Duration::from_millis(2));
    }
    let pid = fs::read_to_string(&pid_path)
        .expect("read escaped fixture PID")
        .parse::<libc::pid_t>()
        .expect("parse escaped fixture PID");
    let identity = process_identity(pid)
        .expect("read escaped fixture identity")
        .expect("escaped fixture remains present before collection");
    let prescanned = fallback_owner
        .identities(Instant::now() + TEST_WAIT)
        .expect("prescan escaped supervisor descendants");
    assert!(
        prescanned.contains(&identity),
        "escaped descendant must be retained before unexpected supervisor exit"
    );
    let pinned = pin_process(identity)
        .expect("pin prescanned escaped descendant")
        .expect("prescanned escaped descendant remains present");
    fs::write(&release_path, []).expect("release escaped intermediate");
    let spawned = SpawnedOutputChild {
        child,
        supervised: true,
        cleanup_proof: Some(completion_reader),
        fallback_owner: Some(fallback_owner),
    };

    let error = collect_output_child_until(spawned, Instant::now() + TEST_WAIT, 1024, 1024, 1024)
        .expect_err("missing authenticated completion must fail supervision");
    assert_eq!(
        error
            .downcast_ref::<DirtyCheckoutError>()
            .expect("typed unexpected supervisor exit error")
            .kind(),
        DirtyCheckoutErrorKind::ResourceUnavailable
    );

    let still_running = {
        let exit_deadline = Instant::now() + TEST_WAIT;
        loop {
            let state = fs::read(format!("/proc/{pid}/stat"))
                .ok()
                .and_then(|bytes| {
                    bytes
                        .iter()
                        .rposition(|byte| *byte == b')')
                        .and_then(|end| bytes.get(end + 2).copied())
                });
            if state.is_none() || state == Some(b'Z') {
                break false;
            }
            if Instant::now() >= exit_deadline {
                break true;
            }
            std::thread::sleep(Duration::from_millis(2));
        }
    };
    #[cfg(target_os = "macos")]
    let still_running = {
        let exit_deadline = Instant::now() + TEST_WAIT;
        loop {
            if process_identity(pid).expect("recheck escaped descendant") != Some(identity) {
                break false;
            }
            if Instant::now() >= exit_deadline {
                break true;
            }
            std::thread::sleep(Duration::from_millis(2));
        }
    };
    if still_running {
        let _ = signal_pinned_process(&pinned, libc::SIGKILL);
        panic!("escaped descendant survived missing-completion fallback cleanup");
    }
}

#[cfg(target_os = "linux")]
#[test]
fn lost_trackers_fail_unavailable_without_signaling_an_unseen_descendant() {
    use std::os::unix::net::UnixStream;

    let root = tempfile::TempDir::new().expect("lost tracker characterization root");
    let pid_path = root.path().join("escaped.pid");
    let release_path = root.path().join("release");
    let (completion_reader, completion_writer) =
        UnixStream::pair().expect("lost tracker completion socket");
    drop(completion_writer);
    let mut command = Command::new(env::current_exe().expect("current test executable"));
    command
        .arg("output_cleanup_escaped_descendant_test_fixture")
        .arg("--nocapture")
        .env("NILS_OUTPUT_CLEANUP_ESCAPED_DESCENDANT", "1")
        .env("NILS_OUTPUT_CLEANUP_EXIT_AFTER_PUBLISH", "1")
        .env("NILS_OUTPUT_CLEANUP_HARD_EXIT_AFTER_PUBLISH", "1")
        .env("NILS_OUTPUT_CLEANUP_RELEASE_PATH", &release_path)
        .env("NILS_PROCESS_SUPERVISOR_TEST_PID_PATH", &pid_path)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .process_group(0);
    let mut child = command.spawn().expect("spawn hard-exit supervisor fixture");
    let supervisor_pid = child.id() as libc::pid_t;
    let fallback_owner =
        ProcessOwner::for_root(supervisor_pid).expect("create unscanned fallback owner");

    let mut canary_command = Command::new("/bin/sleep");
    canary_command
        .arg("60")
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .process_group(supervisor_pid);
    let canary_child = canary_command
        .spawn()
        .expect("spawn unrelated process-group canary");
    let mut canary = SupervisorFixture {
        child: canary_child,
        completion_channel: None,
        reaped: false,
    };
    let canary_pid = canary.id() as libc::pid_t;
    let canary_identity = process_identity(canary_pid)
        .expect("read canary identity")
        .expect("canary remains present after spawn");
    let mut canary_cleanup = PublishedPidCleanup::new(canary_identity);
    assert!(
        canary_cleanup.process().is_some(),
        "test cleanup must pin the canary generation"
    );

    let publication_deadline = Instant::now() + TEST_WAIT;
    while !pid_path.exists() {
        assert!(
            Instant::now() < publication_deadline,
            "escaped fixture did not publish its PID"
        );
        std::thread::sleep(Duration::from_millis(2));
    }
    let leaf_pid = fs::read_to_string(&pid_path)
        .expect("read unseen leaf PID")
        .parse::<libc::pid_t>()
        .expect("parse unseen leaf PID");
    let leaf_identity = process_identity(leaf_pid)
        .expect("read unseen leaf identity")
        .expect("unseen leaf remains present before tracker loss");
    let mut test_only_leaf_reaper = PublishedPidCleanup::new(leaf_identity);
    assert!(
        test_only_leaf_reaper.process().is_some(),
        "test-only cleanup must pin the escaped leaf generation"
    );
    assert!(
        fallback_owner.known_identities().is_empty(),
        "the caller fallback owner must not prescan the escaped leaf"
    );

    fs::write(&release_path, []).expect("release hard-exit supervisor fixture");
    let supervisor_status = child.wait().expect("reap hard-exit supervisor fixture");
    assert_eq!(supervisor_status.code(), Some(exit::RUNTIME));
    assert_eq!(
        process_identity(supervisor_pid).expect("recheck supervisor identity"),
        None,
        "the supervisor generation must be gone before fallback collection"
    );

    reset_unpinned_cleanup_signal_attempts();
    let spawned = SpawnedOutputChild {
        child,
        supervised: true,
        cleanup_proof: Some(completion_reader),
        fallback_owner: Some(fallback_owner),
    };
    let collection_started = Instant::now();
    let error = collect_output_child_until(
        spawned,
        collection_started + TEST_WAIT,
        16 * 1024,
        16 * 1024,
        32 * 1024,
    )
    .expect_err("lost tracking authorities must fail unavailable");
    assert!(
        collection_started.elapsed() < TEST_WAIT / 2,
        "missing cleanup proof must fail within the bounded cleanup window"
    );
    assert_eq!(
        error
            .downcast_ref::<DirtyCheckoutError>()
            .expect("typed lost tracker error")
            .kind(),
        DirtyCheckoutErrorKind::ResourceUnavailable
    );
    assert!(
        error
            .to_string()
            .contains("process supervisor completion could not be authenticated"),
        "the caller must not accept missing authenticated cleanup proof"
    );
    assert_eq!(
        unpinned_cleanup_signal_attempts(),
        0,
        "a reaped supervisor must disable every unpinned cleanup signal path"
    );

    assert_eq!(
        process_identity(canary_pid).expect("recheck canary identity"),
        Some(canary_identity),
        "cleanup must not replace or terminate the unrelated canary generation"
    );
    let canary_stat =
        fs::read(format!("/proc/{canary_pid}/stat")).expect("read canary process state");
    let canary_state = canary_stat
        .iter()
        .rposition(|byte| *byte == b')')
        .and_then(|end| canary_stat.get(end + 2).copied())
        .expect("parse canary process state");
    assert!(
        !matches!(canary_state, b'T' | b't' | b'Z'),
        "cleanup must not stop or terminate the unrelated process-group canary"
    );
    assert!(
        signal_pinned_process(canary_cleanup.process().expect("pinned canary handle"), 0,)
            .expect("probe pinned canary"),
        "the unrelated canary must remain alive"
    );
    assert_eq!(
        process_identity(leaf_pid).expect("recheck unseen leaf identity"),
        Some(leaf_identity),
        "no termination claim exists for a leaf unseen before every tracker was lost"
    );
    assert!(
        signal_pinned_process(
            test_only_leaf_reaper
                .process()
                .expect("pinned test-only leaf handle"),
            0,
        )
        .expect("probe pinned unseen leaf"),
        "the accepted residual leaves the unseen generation for test-only cleanup"
    );

    test_only_leaf_reaper
        .terminate_and_wait()
        .expect("terminate and observe the unseen leaf through its stable test handle");
    canary_cleanup
        .terminate_and_wait()
        .expect("terminate and observe the canary through its stable test handle");
    canary.wait().expect("reap unrelated canary");
}
