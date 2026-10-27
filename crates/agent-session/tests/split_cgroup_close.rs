#![cfg(target_os = "linux")]

use pretty_assertions::{assert_eq, assert_ne};
use std::fs;
use std::os::unix::{fs::PermissionsExt, process::CommandExt};
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};

struct Runtime(Child);
impl Drop for Runtime {
    fn drop(&mut self) {
        // Only this fixture's dedicated process group is signaled by cleanup.
        unsafe { libc::kill(-(self.0.id() as i32), libc::SIGKILL) };
        let _ = self.0.wait();
    }
}

#[test]
fn split_cgroup_close_names_failed_check_without_signaling() {
    split_cgroup_close(false);
}

#[test]
fn exited_captured_helper_keeps_verification_retryable() {
    split_cgroup_close(true);
}

#[test]
fn explicit_recovery_terminates_the_captured_split_scope_runtime() {
    split_cgroup_recovery();
}

#[test]
fn explicit_recovery_refuses_a_changed_captured_member_without_signaling() {
    split_cgroup_fixture(true, true, false);
}

#[test]
fn explicit_recovery_retains_state_when_a_split_member_forks_after_snapshot() {
    split_cgroup_fixture(false, true, true);
}

fn split_cgroup_close(helper_exits: bool) {
    split_cgroup_fixture(helper_exits, false, false);
}

fn split_cgroup_recovery() {
    split_cgroup_fixture(false, true, false);
}

fn split_cgroup_fixture(helper_exits: bool, recover_split_cgroup: bool, add_late_member: bool) {
    let tmp = tempfile::tempdir().unwrap();
    let child_file = tmp.path().join("child");
    let late_trigger = tmp.path().join("late-trigger");
    let late_child_file = tmp.path().join("late-child");
    let scope = format!("tmux-spawn-{}", uuid::Uuid::new_v4());
    // Model an older launcher: a helper keeps the original cgroup while the
    // pane enters a dedicated scope. Both retain the same process session.
    let mut command = Command::new("sh");
    let script = if add_late_member {
        "sh -c 'while [ ! -f \"$1\" ]; do :; done; sleep 60 & echo $! > \"$2\"; wait' helper \"$2\" \"$3\" & echo $! > \"$1\"; exec systemd-run --user --scope --quiet --collect --unit=\"$4\" sleep 60"
    } else {
        "sleep 60 & echo $! > \"$1\"; exec systemd-run --user --scope --quiet --collect --unit=\"$4\" sleep 60"
    };
    command
        .args(["-c", script, "fixture"])
        .arg(&child_file)
        .arg(&late_trigger)
        .arg(&late_child_file)
        .arg(&scope)
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    unsafe {
        command.pre_exec(|| {
            if libc::setsid() < 0 {
                Err(std::io::Error::last_os_error())
            } else {
                Ok(())
            }
        });
    }
    let mut runtime = Runtime(command.spawn().unwrap());
    let pid = runtime.0.id();
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        let cgroup = fs::read_to_string(format!("/proc/{pid}/cgroup")).unwrap_or_default();
        if cgroup.contains(&scope) {
            break;
        }
        assert!(
            runtime.0.try_wait().unwrap().is_none(),
            "fixture requires a systemd user manager"
        );
        assert!(Instant::now() < deadline, "fixture did not enter its scope");
        std::thread::sleep(Duration::from_millis(10));
    }
    let child: i32 = fs::read_to_string(&child_file)
        .unwrap()
        .trim()
        .parse()
        .unwrap();
    assert_eq!(unsafe { libc::getsid(child) }, pid as i32);
    assert_ne!(
        fs::read_to_string(format!("/proc/{pid}/cgroup")).unwrap(),
        fs::read_to_string(format!("/proc/{child}/cgroup")).unwrap()
    );

    let state = tmp.path().join("state");
    let session = state.join("sessions/split-runtime");
    fs::create_dir_all(&session).unwrap();
    fs::write(session.join("session.json"), serde_json::to_vec(&serde_json::json!({
        "schema_version": "agent-session.session.v1", "id": "split-runtime", "agent": "codex",
        "mode": "interactive", "tmux_session": "hs-codex-split-runtime", "cwd": tmp.path(),
        "created_at": "2026-01-01T00:00:00Z", "updated_at": "2026-01-01T00:00:00Z",
        "runtime": {"kind": "tmux", "tmux_session": "hs-codex-split-runtime", "launch_id": "fixture-launch", "generation": 1, "started_at": "2026-01-01T00:00:00Z"}
    })).unwrap()).unwrap();
    let calls = tmp.path().join("calls");
    let tmux = tmp.path().join("tmux");
    let capture_seen = tmp.path().join("capture-seen");
    let helper_exit = if helper_exits {
        // The second identity probe runs after process-session capture. End
        // only the fixture helper there, before membership verification.
        format!(
            r#"if [ -f {seen} ]; then
  kill -KILL {child}
  attempts=0
  while [ -r /proc/{child}/stat ] && [ "$attempts" -lt 100 ]; do
    read -r ignored_pid ignored_name state ignored_rest < /proc/{child}/stat || break
    [ "$state" = Z ] && break
    attempts=$((attempts + 1))
    sleep 0.001
  done
else
  : > {seen}
fi
"#,
            seen = shell_words::quote(capture_seen.to_str().unwrap())
        )
    } else {
        String::new()
    };
    let late_member_probe = if add_late_member {
        format!(
            r#"if [ -f {seen} ]; then
  : > {trigger}
  attempts=0
  while [ ! -s {late_child} ] && [ "$attempts" -lt 100 ]; do
    attempts=$((attempts + 1))
    sleep 0.01
  done
  [ -s {late_child} ] || exit 43
else
  : > {seen}
fi
"#,
            seen = shell_words::quote(capture_seen.to_str().unwrap()),
            trigger = shell_words::quote(late_trigger.to_str().unwrap()),
            late_child = shell_words::quote(late_child_file.to_str().unwrap())
        )
    } else {
        String::new()
    };
    fs::write(
        &tmux,
        format!(
            r#"#!/bin/sh
printf '%s\n' "$1" >> {calls}
case "$1" in
 display-message) {helper_exit}{late_member_probe}printf '$91 %%91 {pid}\n' ;;
 show-environment) case "$4" in
 AGENT_SESSION_ID) echo 'AGENT_SESSION_ID=split-runtime' ;;
 AGENT_SESSION_STATE_DIR) printf 'AGENT_SESSION_STATE_DIR=%s\n' {state} ;;
 AGENT_SESSION_RUNTIME_ID) echo 'AGENT_SESSION_RUNTIME_ID=fixture-launch' ;;
 esac ;;
 has-session) if [ -f {stopped} ]; then echo "can't find session:" >&2; exit 1; fi; exit 0 ;;
 if-shell) : > {stopped}; exit 0 ;;
 *) exit 42 ;;
esac
"#,
            calls = shell_words::quote(calls.to_str().unwrap()),
            state = shell_words::quote(state.to_str().unwrap()),
            stopped = shell_words::quote(tmp.path().join("stopped").to_str().unwrap()),
            late_member_probe = late_member_probe
        ),
    )
    .unwrap();
    fs::set_permissions(&tmux, fs::Permissions::from_mode(0o700)).unwrap();
    let mut command = Command::new(env!("CARGO_BIN_EXE_agent-session"));
    nils_test_support::cmd::strip_ambient_managed_session_env(&mut command);
    command
        .args(["--state-dir"])
        .arg(&state)
        .env("AGENT_SESSION_TMUX_BIN", &tmux);
    if recover_split_cgroup {
        command.args([
            "delete",
            "split-runtime",
            "--recover-split-cgroup",
            "--format",
            "json",
        ]);
    } else {
        command.args(["delete", "split-runtime", "--format", "json"]);
    }
    let output = command.output().unwrap();
    let value: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    if recover_split_cgroup {
        if helper_exits {
            assert!(!output.status.success(), "{value}");
            assert_eq!(unsafe { libc::kill(pid as i32, 0) }, 0);
            assert!(session.join("session.json").is_file());
            assert!(!fs::read_to_string(calls).unwrap().contains("if-shell"));
            return;
        }
        if add_late_member {
            assert!(!output.status.success(), "{value}");
            assert!(session.join("session.json").is_file(), "{value}");
            let late_pid: i32 = fs::read_to_string(&late_child_file)
                .expect("late process was forked")
                .trim()
                .parse()
                .unwrap();
            assert_eq!(unsafe { libc::kill(late_pid, 0) }, 0);
            assert_eq!(unsafe { libc::kill(pid as i32, 0) }, 0);
            assert_eq!(unsafe { libc::kill(child, 0) }, 0);
            assert!(!fs::read_to_string(calls).unwrap().contains("if-shell"));
            return;
        }
        assert!(output.status.success(), "{value}");
        assert!(!session.join("session.json").exists(), "{value}");
        assert!(fs::read_to_string(calls).unwrap().contains("if-shell"));
        for process in [pid as i32, child] {
            let deadline = Instant::now() + Duration::from_secs(5);
            loop {
                let stopped = fs::read_to_string(format!("/proc/{process}/stat"))
                    .map(|stat| {
                        stat.rsplit_once(')')
                            .unwrap()
                            .1
                            .trim_start()
                            .starts_with('Z')
                    })
                    .unwrap_or(true);
                if stopped {
                    break;
                }
                assert!(Instant::now() < deadline, "process {process} remained live");
                std::thread::sleep(Duration::from_millis(10));
            }
        }
        return;
    }
    assert!(!output.status.success(), "{value}");
    assert_eq!(unsafe { libc::kill(pid as i32, 0) }, 0);
    if helper_exits {
        let helper_stopped = fs::read_to_string(format!("/proc/{child}/stat"))
            .map(|stat| {
                stat.rsplit_once(')')
                    .unwrap()
                    .1
                    .trim_start()
                    .starts_with('Z')
            })
            .unwrap_or(true);
        assert!(helper_stopped, "fixture helper must exit after capture");
    } else {
        assert_eq!(unsafe { libc::kill(child, 0) }, 0);
    }
    assert!(!fs::read_to_string(calls).unwrap().contains("if-shell"));
    assert!(session.join("session.json").is_file());
    assert_eq!(value["error"]["code"], "session-termination-failed");
    if helper_exits {
        assert_eq!(value["error"]["details"]["reason"], "verification-failed");
        assert_eq!(value["error"]["details"]["retryable"], true);
        assert_eq!(value["error"]["details"]["action"], "retry-delete");
        assert!(value["error"]["details"]["check"].is_null());
        return;
    }
    assert_eq!(
        value["error"]["details"]["reason"],
        "runtime-identity-mismatch"
    );
    assert!(
        value["error"]["hint"]
            .as_str()
            .unwrap()
            .contains("delete --leave-processes")
    );
    assert_eq!(
        value["error"]["details"]["check"],
        "process-session-member-cgroup"
    );
    assert_eq!(value["error"]["details"]["retryable"], false);
    assert_eq!(
        value["error"]["details"]["action"],
        "inspect-processes-or-delete-with-leave-processes"
    );
    assert!(
        value["error"]["message"]
            .as_str()
            .unwrap()
            .contains("process-session member is outside the pane's recorded cgroup")
    );
}
