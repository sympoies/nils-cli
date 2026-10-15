#[cfg(target_os = "linux")]
use super::process_supervisor::spawn_authenticated_supervisor_output_fixture;
#[cfg(target_os = "linux")]
use super::process_supervisor::{
    PublishedPidCleanup, spawn_authenticated_supervisor_fixture,
    spawn_authenticated_supervisor_fixture_with_timeout,
    spawn_authenticated_supervisor_fixture_with_timeout_and_delay,
};
use super::*;

#[test]
fn direct_supervisor_activation_cannot_execute_an_arbitrary_program() {
    let root = tempfile::TempDir::new().expect("supervisor activation root");
    let marker = root.path().join("executed");
    let executable = snapshot_worker_executable().expect("trusted git-cli executable");
    let status = Command::new(&executable.source_path)
        .env(PROCESS_SUPERVISOR_ENV, "1")
        .args([
            OsStr::new("/bin/sh"),
            OsStr::new("-c"),
            OsStr::new("printf executed > \"$NILS_SUPERVISOR_MARKER\""),
        ])
        .env("NILS_SUPERVISOR_MARKER", &marker)
        .status()
        .expect("invoke direct supervisor activation");

    assert!(
        !status.success(),
        "unauthenticated supervisor mode must fail"
    );
    assert!(!marker.exists(), "unauthenticated argv must never execute");
}

#[cfg(target_os = "linux")]
#[test]
fn linux_process_stat_parser_accepts_non_utf8_process_names() {
    let mut stat = b"4242 (process-\xff-name) S 7".to_vec();
    for _ in 0..17 {
        stat.extend_from_slice(b" 0");
    }
    stat.extend_from_slice(b" 12345\n");

    let record = parse_linux_process_record(&stat)
        .expect("non-UTF-8 process names must not hide process identity");
    assert_eq!(record.identity.pid, 4242);
    assert_eq!(record.parent, 7);
    assert_eq!(record.identity.generation, 12345);
}

pub(super) fn assert_process_absent(pid: libc::pid_t, label: &str) {
    let deadline = Instant::now() + Duration::from_millis(500);
    loop {
        let status = unsafe { libc::kill(pid, 0) };
        if status == -1 && io::Error::last_os_error().raw_os_error() == Some(libc::ESRCH) {
            return;
        }
        assert!(
            Instant::now() < deadline,
            "{label} process {pid} is still present"
        );
        std::thread::sleep(Duration::from_millis(5));
    }
}

#[test]
fn snapshot_worker_process_is_reaped_at_the_hard_deadline() {
    let root = tempfile::TempDir::new().expect("snapshot worker root");
    let pid_path = root.path().join("worker.pid");
    let child_pid_path = root.path().join("child.pid");
    let script = format!(
        "printf %s $$ > '{}'; sleep 60 & printf %s $! > '{}'; while :; do :; done",
        pid_path.display(),
        child_pid_path.display()
    );
    let mut command = Command::new("/bin/sh");
    command
        .args(["-c", &script])
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .process_group(0);

    let error = run_snapshot_worker_command(
        &mut command,
        deadline_after(Duration::from_millis(100)).expect("worker deadline"),
    )
    .expect_err("blocked snapshot worker must hit the hard deadline");

    assert_eq!(
        error
            .downcast_ref::<DirtyCheckoutError>()
            .expect("typed snapshot timeout")
            .kind(),
        DirtyCheckoutErrorKind::Timeout
    );
    let pid: libc::pid_t = fs::read_to_string(pid_path)
        .expect("read worker pid")
        .parse()
        .expect("parse worker pid");
    let child_pid: libc::pid_t = fs::read_to_string(child_pid_path)
        .expect("read worker child pid")
        .parse()
        .expect("parse worker child pid");
    assert_process_absent(pid, "timed-out snapshot worker");
    assert_process_absent(child_pid, "timed-out snapshot worker child");
}

// macOS hosted runners cannot reliably start this real-supervisor kqueue fixture; see #1290.
#[cfg(target_os = "linux")]
#[test]
fn supervisor_ownership_eof_terminates_the_target_before_the_original_deadline() {
    let root = tempfile::TempDir::new().expect("ownership EOF root");
    let pid_path = root.path().join("blocked.pid");
    let mut supervisor = spawn_authenticated_supervisor_fixture_with_timeout(
        "blocked-child",
        &pid_path,
        Duration::from_secs(5),
    );
    let publication_deadline = Instant::now() + Duration::from_secs(1);
    while !pid_path.exists() {
        assert!(
            Instant::now() < publication_deadline,
            "blocked target did not publish its PID"
        );
        std::thread::sleep(Duration::from_millis(2));
    }
    let pid = fs::read_to_string(&pid_path)
        .expect("read blocked target PID")
        .parse::<libc::pid_t>()
        .expect("parse blocked target PID");

    supervisor.abandon_owner();
    let owner_loss_deadline = Instant::now() + Duration::from_millis(500);
    let status = loop {
        if let Some(status) = supervisor.try_wait().expect("poll abandoned supervisor") {
            break status;
        }
        assert!(
            Instant::now() < owner_loss_deadline,
            "supervisor ignored authenticated ownership-channel EOF"
        );
        std::thread::sleep(Duration::from_millis(2));
    };

    assert!(status.success(), "ownership-loss supervisor fixture failed");
    assert_process_absent(pid, "target after supervisor ownership loss");
}

// macOS hosted runners cannot reliably start this real-supervisor kqueue fixture; see #1290.
#[cfg(target_os = "linux")]
#[test]
fn supervisor_retains_the_original_absolute_deadline_after_spawn() {
    let root = tempfile::TempDir::new().expect("supervisor deadline root");
    let pid_path = root.path().join("blocked.pid");
    let started = Instant::now();
    let mut supervisor = spawn_authenticated_supervisor_fixture_with_timeout_and_delay(
        "blocked-child",
        &pid_path,
        Duration::from_millis(500),
        Duration::from_millis(300),
    );
    let completion_deadline = Instant::now() + Duration::from_secs(1);
    let status = loop {
        if let Some(status) = supervisor.try_wait().expect("poll deadline supervisor") {
            break status;
        }
        assert!(
            Instant::now() < completion_deadline,
            "supervisor did not retain the original absolute deadline"
        );
        std::thread::sleep(Duration::from_millis(2));
    };

    assert!(status.success(), "deadline supervisor fixture failed");
    assert!(
        started.elapsed() < Duration::from_millis(700),
        "supervisor reset the deadline after delayed authentication: {:?}",
        started.elapsed()
    );
    let pid = fs::read_to_string(&pid_path)
        .expect("read deadline target PID")
        .parse::<libc::pid_t>()
        .expect("parse deadline target PID");
    assert_process_absent(pid, "target after supervisor deadline");
}

// macOS hosted runners cannot reliably start this four-process kqueue fixture; see #1290.
#[cfg(target_os = "linux")]
#[test]
fn successful_git_output_reaps_a_surviving_process_group_child() {
    let root = tempfile::TempDir::new().expect("surviving child root");
    let pid_path = root.path().join("child.pid");
    let mut supervisor = spawn_authenticated_supervisor_fixture("successful-child", &pid_path);
    let publication_deadline = Instant::now() + Duration::from_secs(1);
    while !pid_path.exists() {
        assert!(
            Instant::now() < publication_deadline,
            "successful child did not publish its process identity"
        );
        std::thread::sleep(Duration::from_millis(2));
    }
    let pid: libc::pid_t = fs::read_to_string(&pid_path)
        .expect("read surviving child pid")
        .parse()
        .expect("parse surviving child pid");
    let identity = process_identity(pid)
        .expect("read surviving child identity")
        .expect("surviving child generation exists");
    let cleanup = PublishedPidCleanup::new(identity);
    let status = supervisor.wait().expect("wait for supervisor fixture");

    assert!(status.success(), "authenticated supervisor fixture failed");
    assert_process_absent(pid, "successful command child");
    drop(cleanup);
}

#[test]
fn git_output_deadline_does_not_join_a_descendant_held_pipe() {
    let mut command = Command::new("/bin/sh");
    command
        .args(["-c", "while :; do :; done & while :; do :; done"])
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .process_group(0);
    let started = Instant::now();

    let error = output_with_limits(&mut command, Duration::from_millis(100), 1024, 1024)
        .expect_err("descendant-held pipe must hit the hard deadline");

    assert!(
        started.elapsed() < Duration::from_secs(2),
        "hard deadline was not enforced: {:?}",
        started.elapsed()
    );
    assert!(error.to_string().contains("time limit"));
}

// Hosted macOS coverage races kqueue ownership with the exiting group leader; see #1290.
#[cfg(target_os = "linux")]
#[test]
fn output_cleanup_tracks_escaped_descendants_after_the_group_leader_exits() {
    let root = tempfile::TempDir::new().expect("escaped output child root");
    let pid_path = root.path().join("escaped.pid");
    let spawned =
        spawn_authenticated_supervisor_output_fixture("escaped-output-timeout", &pid_path);
    let publication_deadline = Instant::now() + Duration::from_secs(1);
    while !pid_path.exists() {
        assert!(
            Instant::now() < publication_deadline,
            "escaped output leaf did not publish its process identity"
        );
        std::thread::sleep(Duration::from_millis(2));
    }
    let pid: libc::pid_t = fs::read_to_string(&pid_path)
        .expect("read escaped output leaf PID")
        .parse()
        .expect("parse escaped output leaf PID");
    let identity = process_identity(pid)
        .expect("read escaped output leaf identity")
        .expect("escaped output leaf generation exists");
    let cleanup = PublishedPidCleanup::new(identity);

    let error = collect_output_child_until(
        spawned,
        Instant::now() + Duration::from_millis(100),
        1024,
        1024,
        1024,
    )
    .expect_err("the parked target must hit the supervised output deadline");
    assert_eq!(
        error
            .downcast_ref::<DirtyCheckoutError>()
            .expect("typed escaped output timeout")
            .kind(),
        DirtyCheckoutErrorKind::Timeout
    );
    assert_process_absent(pid, "escaped output descendant after leader exit");
    drop(cleanup);
}

#[test]
fn git_output_collection_enforces_an_aggregate_stream_budget() {
    let mut command = Command::new("/bin/sh");
    command
            .args([
                "-c",
                "i=0; while [ $i -lt 700 ]; do printf x; i=$((i + 1)); done; i=0; while [ $i -lt 700 ]; do printf y >&2; i=$((i + 1)); done",
            ])
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .process_group(0);

    let error = output_with_limits(&mut command, Duration::from_secs(1), 1024, 1024)
        .expect_err("combined stdout and stderr must share an aggregate budget");

    assert!(error.to_string().contains("size limit"));
}

// macOS hosted runners cannot reliably start this four-process kqueue fixture; see #1290.
#[cfg(target_os = "linux")]
#[test]
fn detached_double_fork_pipe_holder_is_owned_and_terminated() {
    let root = tempfile::TempDir::new().expect("detached child root");
    let pid_path = root.path().join("detached.pid");
    let mut supervisor = spawn_authenticated_supervisor_fixture("detached-double-fork", &pid_path);
    let started = Instant::now();
    while !pid_path.exists() {
        assert!(
            started.elapsed() < Duration::from_secs(1),
            "detached fixture did not publish its process identity"
        );
        std::thread::sleep(Duration::from_millis(2));
    }
    let pid: libc::pid_t = fs::read_to_string(&pid_path)
        .expect("read detached final pid")
        .parse()
        .expect("parse detached final pid");
    let identity = process_identity(pid)
        .expect("read detached final identity")
        .expect("detached final generation exists");
    let cleanup = PublishedPidCleanup::new(identity);
    unsafe {
        libc::kill(-(supervisor.id() as libc::pid_t), libc::SIGTERM);
    }
    let status = supervisor
        .wait()
        .expect("wait for detached supervisor fixture");
    assert!(status.success(), "detached supervisor fixture failed");

    assert_process_absent(pid, "detached double-fork timeout descendant");
    drop(cleanup);
}
