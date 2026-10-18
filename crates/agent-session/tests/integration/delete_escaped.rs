use crate::cli::{fake_agent, run, write_resumable_session_record_with_agent_bin};
use pretty_assertions::assert_eq;
use serde_json::Value;
use std::{
    fs,
    process::{Child, Command, Stdio},
    thread,
    time::{Duration, Instant},
};

struct Daemon(Child);
impl Drop for Daemon {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

#[test]
fn delete_names_escaped_daemon_and_explicitly_leaves_it_running() {
    let tmp = tempfile::tempdir().unwrap();
    let state = tmp.path().join("state");
    let cwd = tmp.path().join("repo");
    fs::create_dir(&cwd).unwrap();
    let tmux = tmp.path().join("tmux");
    let stopped = tmp.path().join("tmux-stopped");
    let agent = fake_agent(tmp.path(), "claude");
    let session = write_resumable_session_record_with_agent_bin(
        &state,
        "escaped-daemon",
        "claude",
        "hs-claude-escaped-daemon",
        &cwd,
        &["--resume", "fixture-conversation"],
        Some(&agent),
    );
    let mut record: Value =
        serde_json::from_slice(&fs::read(session.join("session.json")).unwrap()).unwrap();
    record["runtime"]["launch_id"] = serde_json::json!(uuid::Uuid::new_v4().to_string());
    record
        .as_object_mut()
        .unwrap()
        .remove("tmux_runtime_never_launched");
    fs::write(
        session.join("session.json"),
        serde_json::to_vec(&record).unwrap(),
    )
    .unwrap();
    let mut pane = Daemon(
        Command::new("python3")
            .args(["-c", "import os,time; os.setsid(); time.sleep(60)"])
            .spawn()
            .unwrap(),
    );
    let pane_deadline = Instant::now() + Duration::from_secs(3);
    while unsafe { libc::getsid(pane.0.id() as i32) } != pane.0.id() as i32 {
        assert!(Instant::now() < pane_deadline);
        assert!(pane.0.try_wait().unwrap().is_none());
        thread::sleep(Duration::from_millis(10));
    }
    let script = format!(
        r#"#!/bin/sh
case "$1" in
 display-message) [ ! -f {stopped} ] || {{ echo "can't find session: fixture" >&2; exit 1; }}; printf '$91 %%91 {pid}\n' ;;
 show-environment) case "$4" in
 AGENT_SESSION_ID) echo 'AGENT_SESSION_ID=escaped-daemon' ;;
 AGENT_SESSION_STATE_DIR) printf 'AGENT_SESSION_STATE_DIR=%s\n' {state} ;;
 AGENT_SESSION_RUNTIME_ID) printf 'AGENT_SESSION_RUNTIME_ID=%s\n' {runtime} ;;
 esac ;;
 if-shell) : > {stopped}; kill -KILL {pid} 2>/dev/null || true ;;
 has-session) [ ! -f {stopped} ] || {{ echo "can't find session: fixture" >&2; exit 1; }} ;;
 *) exit 42 ;;
esac
"#,
        stopped = shell_words::quote(stopped.to_str().unwrap()),
        pid = pane.0.id(),
        state = shell_words::quote(state.to_str().unwrap()),
        runtime = shell_words::quote(record["runtime"]["launch_id"].as_str().unwrap())
    );
    fs::write(&tmux, script).unwrap();
    use std::os::unix::fs::PermissionsExt;
    fs::set_permissions(&tmux, fs::Permissions::from_mode(0o700)).unwrap();
    let ready = tmp.path().join("daemon-ready");
    // A detached process carrying the runtime id, outside the fake pane tree.
    // Keep ownership of its child handle so assertion failures also clean up.
    let mut daemon = Daemon(
        Command::new("python3")
            .args([
                "-c",
                "import os,sys,time; os.setsid(); open(sys.argv[1], 'w').close(); time.sleep(60)",
            ])
            .arg(&ready)
            .arg("private-argument-sentinel")
            .env("AGENT_SESSION_ID", "escaped-daemon")
            .env(
                "AGENT_SESSION_RUNTIME_ID",
                record["runtime"]["launch_id"].as_str().unwrap(),
            )
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .unwrap(),
    );
    let deadline = Instant::now() + Duration::from_secs(3);
    while !ready.exists() {
        assert!(Instant::now() < deadline);
        thread::sleep(Duration::from_millis(10));
    }
    let state_arg = state.to_str().unwrap();
    let tmux_arg = tmux.to_str().unwrap();
    let envs = [];
    record["delete_tmux_termination_state"] = serde_json::json!({
        "launch_id": record["runtime"]["launch_id"], "state": "freeze-pending", "thaw_on_recovery": true,
    });
    fs::write(
        session.join("session.json"),
        serde_json::to_vec(&record).unwrap(),
    )
    .unwrap();
    let rejected = run(
        tmp.path(),
        &[
            "--state-dir",
            state_arg,
            "delete",
            "escaped-daemon",
            "--leave-processes",
            "--tmux-bin",
            tmux_arg,
            "--format",
            "json",
        ],
        &envs,
    );
    assert_eq!(rejected.code, 1);
    assert_eq!(
        rejected.stdout_json()["error"]["code"],
        "session-termination-in-progress"
    );
    assert!(session.exists());
    assert!(!stopped.exists());
    record
        .as_object_mut()
        .unwrap()
        .remove("delete_tmux_termination_state");
    fs::write(
        session.join("session.json"),
        serde_json::to_vec(&record).unwrap(),
    )
    .unwrap();
    let original = fs::read_to_string(&tmux).unwrap();
    fs::write(
        &tmux,
        original.replace(
            "AGENT_SESSION_ID=escaped-daemon",
            "AGENT_SESSION_ID=other-session",
        ),
    )
    .unwrap();
    let rejected = run(
        tmp.path(),
        &[
            "--state-dir",
            state_arg,
            "delete",
            "escaped-daemon",
            "--leave-processes",
            "--tmux-bin",
            tmux_arg,
            "--format",
            "json",
        ],
        &envs,
    );
    assert_eq!(rejected.code, 1);
    assert_eq!(
        rejected.stdout_json()["error"]["details"]["reason"],
        "runtime-identity-mismatch"
    );
    assert!(session.exists());
    assert!(!stopped.exists());
    fs::write(&tmux, original).unwrap();
    // Real tmux reaps its pane after killing it. On macOS, the verifier's
    // process-group probe also sees zombies, so reap concurrently with delete.
    #[cfg(target_os = "macos")]
    let pane_reaper = thread::spawn(move || {
        let deadline = Instant::now() + Duration::from_secs(10);
        loop {
            if pane.0.try_wait().unwrap().is_some() {
                break;
            }
            assert!(
                Instant::now() < deadline,
                "fake tmux pane was not terminated"
            );
            thread::sleep(Duration::from_millis(5));
        }
        // Daemon's drop guard also kills/reaps on a timeout panic.
    });
    let output = run(
        tmp.path(),
        &[
            "--state-dir",
            state_arg,
            "delete",
            "escaped-daemon",
            "--tmux-bin",
            tmux_arg,
            "--format",
            "json",
        ],
        &envs,
    );
    #[cfg(target_os = "macos")]
    pane_reaper.join().expect("reap fake tmux pane");
    assert_eq!(output.code, 1, "{}", output.stdout_text());
    let error = output.stdout_json();
    let blockers = error["error"]["details"]["blocking_processes"]
        .as_array()
        .expect("blocking PIDs and commands");
    assert!(
        blockers
            .iter()
            .any(|p| p["pid"] == daemon.0.id() && p["command"].as_str().is_some())
    );
    assert_eq!(error["error"]["details"]["retryable"], false);
    assert!(session.exists());
    assert!(!output.stdout_text().contains("private-argument-sentinel"));
    let output = run(
        tmp.path(),
        &[
            "--state-dir",
            state_arg,
            "delete",
            "escaped-daemon",
            "--leave-processes",
            "--tmux-bin",
            tmux_arg,
            "--format",
            "json",
        ],
        &envs,
    );
    assert_eq!(output.code, 0, "{}", output.stdout_text());
    let result = output.stdout_json();
    assert_eq!(result["data"]["killed"], false);
    assert_eq!(result["data"]["termination_scope"], "tmux-only");
    assert!(
        result["data"]["survivors"]
            .as_array()
            .unwrap()
            .iter()
            .any(|p| p["pid"] == daemon.0.id())
    );
    assert!(!session.exists());
    assert!(daemon.0.try_wait().unwrap().is_none());
    assert!(!output.stdout_text().contains("private-argument-sentinel"));
}
