//! The tmux kill can end every pane process, and the user manager then removes
//! the empty scope before the whole-cgroup kill runs. The pinned directory
//! proves that stop; it must not be reported as an unverifiable termination.
//! A failed kill write on a scope that still exists must keep failing closed.

use crate::*;
use pretty_assertions::assert_eq;
use std::os::unix::process::CommandExt;
use std::process::{Child, Command, Stdio};

fn systemd_scope_runner() -> Option<PathBuf> {
    let required = env::var("AGENT_SESSION_TEST_REQUIRE_CGROUP").as_deref() == Ok("1");
    let runner = binary_on_path("systemd-run");
    let reachable = env::var_os("XDG_RUNTIME_DIR")
        .is_some_and(|dir| Path::new(&dir).join("systemd/private").exists());
    match runner {
        Some(runner) if reachable => Some(runner),
        _ if required => panic!("a systemd user manager is required but unavailable"),
        _ => {
            eprintln!("SKIP: a systemd user manager is unavailable");
            None
        }
    }
}

/// A pane process alone in its own transient tmux-style scope, pinned the way
/// the delete path pins it.
struct ScopedPane {
    pane: Child,
    runtime: PinnedProcessRuntime,
    identity: TmuxRuntimeIdentity,
}

impl Drop for ScopedPane {
    fn drop(&mut self) {
        let _ = self.pane.kill();
        let _ = self.pane.wait();
    }
}

impl ScopedPane {
    fn start(systemd_run: PathBuf) -> Self {
        assert!(
            unsafe { libc::getsid(0) } > 1,
            "cgroup test must run in an isolated POSIX session"
        );
        let unit = format!("tmux-spawn-{}", uuid::Uuid::new_v4());
        let mut command = Command::new(systemd_run);
        command
            .args(["--user", "--scope", "--quiet", "--collect", "--unit"])
            .arg(&unit)
            .args(["--", "sleep", "60"])
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null());
        // A separate POSIX session, as a tmux pane has, keeps the runtime's
        // process-session boundary distinct from the test runner's.
        unsafe {
            command.pre_exec(|| {
                if libc::setsid() < 0 {
                    Err(io::Error::last_os_error())
                } else {
                    Ok(())
                }
            });
        }
        let mut pane = command.spawn().unwrap();
        let pid = pane.id() as libc::pid_t;
        let deadline = Instant::now() + Duration::from_secs(10);
        let control_group = loop {
            match linux_process_control_group(pid) {
                Ok(Some(group)) if group.path.ends_with(&format!("{unit}.scope")) => break group,
                _ if Instant::now() < deadline => thread::sleep(Duration::from_millis(10)),
                other => {
                    let _ = pane.kill();
                    let _ = pane.wait();
                    panic!("the pane never entered its scope: {other:?}");
                }
            }
        };
        let full_path = linux_control_group_full_path(Path::new(&control_group.path)).unwrap();
        let runtime = PinnedProcessRuntime {
            processes: vec![pin_linux_control_group_process(pid, &control_group).unwrap()],
            control_group: Some(
                open_pinned_linux_control_group(&control_group, &full_path).unwrap(),
            ),
        };
        let identity = TmuxRuntimeIdentity {
            macos_boot_id: None,
            launch_id: None,
            session_id: "$91".to_string(),
            pane_id: "%91".to_string(),
            pane_pid: pid,
            pane_start_time: None,
            process_group_id: Some(pid),
            process_session_id: Some(pid),
            process_session_members: Vec::new(),
            control_group_members: Vec::new(),
            control_group: Some(control_group),
            cgroup_mount: None,
            pid_namespace: None,
        };
        Self {
            pane,
            runtime,
            identity,
        }
    }

    fn terminate(&mut self) -> Result<(), SessionTerminationFailure> {
        runtime_teardown::terminate_captured_process_runtime(
            &self.identity,
            &mut self.runtime,
            Duration::from_secs(5),
        )
    }
}

#[test]
fn termination_accepts_a_scope_removed_after_the_tmux_kill() {
    let Some(systemd_run) = systemd_scope_runner() else {
        return;
    };
    let mut scoped = ScopedPane::start(systemd_run);

    // The tmux kill ends the pane; reaping it empties the scope and the user
    // manager removes the scope's directory.
    scoped.pane.kill().unwrap();
    scoped.pane.wait().unwrap();
    let directory_fd = &scoped.runtime.control_group.as_ref().unwrap().directory_fd;
    let removed_at = Instant::now() + Duration::from_secs(10);
    while !pinned_control_group_was_removed(directory_fd).unwrap_or(false) {
        assert!(
            Instant::now() < removed_at,
            "the emptied scope was never removed"
        );
        thread::sleep(Duration::from_millis(10));
    }

    assert_eq!(scoped.terminate(), Ok(()));
}

#[test]
fn termination_fails_closed_when_the_kill_write_fails_on_a_live_scope() {
    let Some(systemd_run) = systemd_scope_runner() else {
        return;
    };
    let mut scoped = ScopedPane::start(systemd_run);
    // A read-only descriptor makes the kill write fail while the scope and its
    // pane are alive: nothing proves the stop, so nothing may be accepted.
    let read_only = fs::File::open("/dev/null").unwrap();
    scoped.runtime.control_group.as_mut().unwrap().kill_fd = read_only.into();

    assert_eq!(
        scoped.terminate(),
        Err(SessionTerminationFailure::VerificationFailed)
    );
    assert!(
        scoped.pane.try_wait().unwrap().is_none(),
        "the pane must not be signaled when its stop is not proven"
    );
}

#[test]
fn only_an_observed_removal_proves_the_scope_stopped() {
    assert!(super::scope_removal_proves_stop(Ok(true)));
    assert!(!super::scope_removal_proves_stop(Ok(false)));
    // A removal state that cannot be read must never count as removed.
    assert!(!super::scope_removal_proves_stop(Err(
        SessionTerminationFailure::VerificationFailed
    )));
}
