//! The tmux kill can end every pane process, and the user manager then removes
//! the empty scope before the whole-cgroup kill runs. The pinned directory
//! proves that stop; it must not be reported as an unverifiable termination.

use crate::*;
use pretty_assertions::assert_eq;
use std::os::unix::process::CommandExt;
use std::process::{Command, Stdio};

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

#[test]
fn termination_accepts_a_scope_removed_after_the_tmux_kill() {
    assert!(
        unsafe { libc::getsid(0) } > 1,
        "cgroup test must run in an isolated POSIX session"
    );
    let Some(systemd_run) = systemd_scope_runner() else {
        return;
    };
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
            Ok(Some(control_group)) if control_group.path.ends_with(&format!("{unit}.scope")) => {
                break control_group;
            }
            _ if Instant::now() < deadline => thread::sleep(Duration::from_millis(10)),
            other => {
                let _ = pane.kill();
                let _ = pane.wait();
                panic!("the pane never entered its scope: {other:?}");
            }
        }
    };
    let full_path = linux_control_group_full_path(Path::new(&control_group.path)).unwrap();
    let mut runtime = PinnedProcessRuntime {
        processes: vec![pin_linux_control_group_process(pid, &control_group).unwrap()],
        control_group: Some(open_pinned_linux_control_group(&control_group, &full_path).unwrap()),
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

    // The tmux kill ends the pane; reaping it empties the scope and the user
    // manager removes the scope's directory.
    pane.kill().unwrap();
    pane.wait().unwrap();
    let directory_fd = &runtime.control_group.as_ref().unwrap().directory_fd;
    let removed_at = Instant::now() + Duration::from_secs(10);
    while !pinned_control_group_was_removed(directory_fd).unwrap_or(false) {
        assert!(
            Instant::now() < removed_at,
            "the emptied scope was never removed"
        );
        thread::sleep(Duration::from_millis(10));
    }

    assert_eq!(
        runtime_teardown::terminate_captured_process_runtime(
            &identity,
            &mut runtime,
            Duration::from_secs(5),
        ),
        Ok(())
    );
}
