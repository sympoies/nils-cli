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
fn legacy_split_cgroup_close_names_failed_check_without_signaling() {
    let tmp = tempfile::tempdir().unwrap();
    let child_file = tmp.path().join("child");
    let scope = format!("tmux-spawn-{}", uuid::Uuid::new_v4());
    // Model an older launcher: a helper keeps the original cgroup while the
    // pane enters a dedicated scope. Both retain the same process session.
    let mut command = Command::new("sh");
    command.args(["-c", "sleep 60 & echo $! > \"$1\"; exec systemd-run --user --scope --quiet --collect --unit=\"$2\" sleep 60", "fixture"])
        .arg(&child_file).arg(&scope)
        .stdout(Stdio::null()).stderr(Stdio::null());
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
    let session = state.join("sessions/legacy-runtime");
    fs::create_dir_all(&session).unwrap();
    fs::write(session.join("session.json"), serde_json::to_vec(&serde_json::json!({
        "schema_version": "agent-session.session.v1", "id": "legacy-runtime", "agent": "codex",
        "mode": "interactive", "tmux_session": "hs-codex-legacy-runtime", "cwd": tmp.path(),
        "created_at": "2026-01-01T00:00:00Z", "updated_at": "2026-01-01T00:00:00Z",
        "runtime": {"kind": "tmux", "tmux_session": "hs-codex-legacy-runtime", "launch_id": "fixture-launch", "generation": 1, "started_at": "2026-01-01T00:00:00Z"}
    })).unwrap()).unwrap();
    let calls = tmp.path().join("calls");
    let tmux = tmp.path().join("tmux");
    fs::write(
        &tmux,
        format!(
            r#"#!/bin/sh
printf '%s\n' "$1" >> {calls}
case "$1" in
 display-message) printf '$91 %%91 {pid}\n' ;;
 show-environment) case "$4" in
 AGENT_SESSION_ID) echo 'AGENT_SESSION_ID=legacy-runtime' ;;
 AGENT_SESSION_STATE_DIR) printf 'AGENT_SESSION_STATE_DIR=%s\n' {state} ;;
 AGENT_SESSION_RUNTIME_ID) echo 'AGENT_SESSION_RUNTIME_ID=fixture-launch' ;;
 esac ;;
 has-session) exit 0 ;;
 *) exit 42 ;;
esac
"#,
            calls = shell_words::quote(calls.to_str().unwrap()),
            state = shell_words::quote(state.to_str().unwrap())
        ),
    )
    .unwrap();
    fs::set_permissions(&tmux, fs::Permissions::from_mode(0o700)).unwrap();
    let mut command = Command::new(env!("CARGO_BIN_EXE_agent-session"));
    nils_test_support::cmd::strip_ambient_managed_session_env(&mut command);
    let output = command
        .args(["--state-dir"])
        .arg(&state)
        .env("AGENT_SESSION_TMUX_BIN", &tmux)
        .args(["delete", "legacy-runtime", "--format", "json"])
        .output()
        .unwrap();
    let value: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert!(!output.status.success(), "{value}");
    assert_eq!(unsafe { libc::kill(pid as i32, 0) }, 0);
    assert_eq!(unsafe { libc::kill(child, 0) }, 0);
    assert!(!fs::read_to_string(calls).unwrap().contains("if-shell"));
    assert!(session.join("session.json").is_file());
    assert_eq!(value["error"]["code"], "session-termination-failed");
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
