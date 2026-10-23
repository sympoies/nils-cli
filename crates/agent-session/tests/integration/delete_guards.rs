use crate::cli::{fake_agent, run, write_resumable_session_record_with_agent_bin};
use pretty_assertions::assert_eq;
use serde_json::{Value, json};
use std::{
    fs,
    os::unix::fs::PermissionsExt,
    path::PathBuf,
    process::{Child, Command, Stdio},
    thread,
    time::{Duration, Instant},
};

struct Pane(Child);
impl Drop for Pane {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

struct Fixture {
    _tmp: tempfile::TempDir,
    state: PathBuf,
    session: PathBuf,
    tmux: PathBuf,
    killed: PathBuf,
    name_status: PathBuf,
    status_entered: PathBuf,
    status_release: PathBuf,
    pane: Pane,
    record: Value,
}

impl Fixture {
    fn new() -> Self {
        let tmp = tempfile::tempdir().unwrap();
        let state = tmp.path().join("state");
        let agent = fake_agent(tmp.path(), "claude");
        let session = write_resumable_session_record_with_agent_bin(
            &state,
            "guarded-delete",
            "claude",
            "hs-claude-guarded-delete",
            tmp.path(),
            &["--resume", "fixture-conversation"],
            Some(&agent),
        );
        let mut record: Value =
            serde_json::from_slice(&fs::read(session.join("session.json")).unwrap()).unwrap();
        record["runtime"]["launch_id"] = json!(uuid::Uuid::new_v4().to_string());
        record
            .as_object_mut()
            .unwrap()
            .remove("tmux_runtime_never_launched");
        fs::write(
            session.join("session.json"),
            serde_json::to_vec(&record).unwrap(),
        )
        .unwrap();
        let mut pane = Pane(
            Command::new("python3")
                .args(["-c", "import os,time; os.setsid(); time.sleep(60)"])
                .spawn()
                .unwrap(),
        );
        let deadline = Instant::now() + Duration::from_secs(3);
        while unsafe { libc::getsid(pane.0.id() as i32) } != pane.0.id() as i32 {
            assert!(Instant::now() < deadline);
            assert!(pane.0.try_wait().unwrap().is_none());
            thread::sleep(Duration::from_millis(10));
        }
        let tmux = tmp.path().join("tmux");
        let killed = tmp.path().join("kill-attempted");
        let name_status = tmp.path().join("name-status");
        let status_entered = tmp.path().join("status-entered");
        let status_release = tmp.path().join("status-release");
        fs::write(&name_status, "stopped").unwrap();
        // Numeric target absence and managed-name absence are separate answers.
        // A successful conditional command need not remove a replacement name.
        let script = format!(
            r#"#!/bin/sh
absent() {{ echo "can't find session: fixture" >&2; exit 1; }}
case "$1" in
 display-message) printf '$91 %%91 {pid}\n' ;;
 show-environment) case "$4" in
 AGENT_SESSION_ID) echo 'AGENT_SESSION_ID=guarded-delete' ;;
 AGENT_SESSION_STATE_DIR) printf 'AGENT_SESSION_STATE_DIR=%s\n' {state} ;;
 AGENT_SESSION_RUNTIME_ID) printf 'AGENT_SESSION_RUNTIME_ID=%s\n' {runtime} ;;
 esac ;;
 if-shell) : > {killed} ;;
 has-session)
   [ "$3" != '$91' ] || {{ [ -f {killed} ] && absent; exit 0; }}
   [ "$3" = '=hs-claude-guarded-delete' ] || exit 42
   case "$(cat {name_status})" in
     running) exit 0 ;; unknown) exit 42 ;; stopped) absent ;;
     stopped-wait) : > {status_entered}; while [ ! -f {status_release} ]; do sleep 0.01; done; absent ;;
     *) exit 42 ;;
   esac ;;
 *) exit 42 ;;
esac
"#,
            pid = pane.0.id(),
            state = shell_words::quote(state.to_str().unwrap()),
            runtime = shell_words::quote(record["runtime"]["launch_id"].as_str().unwrap()),
            killed = shell_words::quote(killed.to_str().unwrap()),
            name_status = shell_words::quote(name_status.to_str().unwrap()),
            status_entered = shell_words::quote(status_entered.to_str().unwrap()),
            status_release = shell_words::quote(status_release.to_str().unwrap()),
        );
        fs::write(&tmux, script).unwrap();
        fs::set_permissions(&tmux, fs::Permissions::from_mode(0o700)).unwrap();
        Self {
            _tmp: tmp,
            state,
            session,
            tmux,
            killed,
            name_status,
            status_entered,
            status_release,
            pane,
            record,
        }
    }

    fn delete(&self) -> nils_test_support::cmd::CmdOutput {
        run(
            self._tmp.path(),
            &[
                "--state-dir",
                self.state.to_str().unwrap(),
                "delete",
                "guarded-delete",
                "--leave-processes",
                "--tmux-bin",
                self.tmux.to_str().unwrap(),
                "--format",
                "json",
            ],
            &[],
        )
    }

    fn delete_if_stopped(&self, incarnation: &str) -> nils_test_support::cmd::CmdOutput {
        run(
            self._tmp.path(),
            &[
                "--state-dir",
                self.state.to_str().unwrap(),
                "delete",
                "guarded-delete",
                "--if-stopped",
                "--expected-incarnation",
                incarnation,
                "--tmux-bin",
                self.tmux.to_str().unwrap(),
                "--format",
                "json",
            ],
            &[],
        )
    }

    fn mark_runtime_stopped(&mut self) -> String {
        self.pane.0.kill().unwrap();
        self.pane.0.wait().unwrap();
        let incarnation = self.record["runtime"]["launch_id"]
            .as_str()
            .unwrap()
            .to_owned();
        let mut record = self.record.clone();
        record["delete_tmux_identity"] = json!({
            "launch_id": incarnation,
            "session_id": "$91",
            "pane_id": "%91",
            "pane_pid": self.pane.0.id(),
            "process_group_id": self.pane.0.id(),
        });
        fs::write(
            self.session.join("session.json"),
            serde_json::to_vec(&record).unwrap(),
        )
        .unwrap();
        incarnation
    }
}

#[test]
fn stopped_only_delete_fences_incarnation_and_refuses_unproven_state() {
    let mut fixture = Fixture::new();
    let incarnation = fixture.record["runtime"]["launch_id"]
        .as_str()
        .unwrap()
        .to_owned();
    for (status, expected_code) in [
        ("running", "session-state-mismatch"),
        ("unknown", "session-status-unknown"),
    ] {
        fs::write(&fixture.name_status, status).unwrap();
        let output = fixture.delete_if_stopped(&incarnation);
        assert_ne!(output.code, 0, "{status}: {}", output.stdout_text());
        assert_eq!(output.stdout_json()["error"]["code"], expected_code);
        assert!(
            !fixture.killed.exists(),
            "conditional delete must not signal"
        );
        assert!(fixture.session.exists());
    }

    let output = fixture.delete_if_stopped("replaced-incarnation");
    assert_ne!(output.code, 0, "{}", output.stdout_text());
    assert_eq!(
        output.stdout_json()["error"]["code"],
        "session-incarnation-conflict"
    );
    assert!(!fixture.killed.exists());
    assert!(fixture.session.exists());

    let mut working_record = fixture.record.clone();
    working_record["delete_tmux_identity"] = json!({
        "launch_id": incarnation,
        "session_id": "$91",
        "pane_id": "%91",
        "pane_pid": fixture.pane.0.id(),
        "process_group_id": fixture.pane.0.id(),
    });
    fs::write(
        fixture.session.join("session.json"),
        serde_json::to_vec(&working_record).unwrap(),
    )
    .unwrap();
    fs::write(&fixture.name_status, "stopped").unwrap();
    let output = fixture.delete_if_stopped(&incarnation);
    assert_ne!(output.code, 0, "{}", output.stdout_text());
    assert_eq!(
        output.stdout_json()["error"]["code"],
        "session-state-mismatch"
    );
    assert_eq!(output.stdout_json()["error"]["details"]["state"], "working");
    assert!(!fixture.killed.exists());

    fixture.mark_runtime_stopped();
    fs::write(&fixture.name_status, "stopped").unwrap();
    let output = fixture.delete_if_stopped(&incarnation);
    assert_eq!(output.code, 0, "{}", output.stdout_text());
    assert_eq!(output.stdout_json()["data"]["deleted"], true);
    assert!(
        !fixture.killed.exists(),
        "conditional delete must not signal"
    );
    assert!(!fixture.session.exists());
}

#[test]
fn stopped_only_delete_serializes_a_concurrent_resume_through_removal() {
    let mut fixture = Fixture::new();
    let incarnation = fixture.mark_runtime_stopped();
    fs::write(&fixture.name_status, "stopped-wait").unwrap();

    let delete = Command::new(nils_test_support::bin::resolve("agent-session"))
        .args([
            "--state-dir",
            fixture.state.to_str().unwrap(),
            "delete",
            "guarded-delete",
            "--if-stopped",
            "--expected-incarnation",
            &incarnation,
            "--tmux-bin",
            fixture.tmux.to_str().unwrap(),
            "--format",
            "json",
        ])
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    let deadline = Instant::now() + Duration::from_secs(5);
    while !fixture.status_entered.exists() {
        assert!(
            Instant::now() < deadline,
            "delete did not reach stopped probe"
        );
        thread::sleep(Duration::from_millis(5));
    }

    let mut resume = Command::new(nils_test_support::bin::resolve("agent-session"))
        .args([
            "--state-dir",
            fixture.state.to_str().unwrap(),
            "resume",
            "guarded-delete",
            "--tmux-bin",
            fixture.tmux.to_str().unwrap(),
            "--format",
            "json",
        ])
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    thread::sleep(Duration::from_millis(50));
    assert!(
        resume.try_wait().unwrap().is_none(),
        "resume escaped the delete lock"
    );

    fs::write(&fixture.status_release, "release").unwrap();
    let deleted = delete.wait_with_output().unwrap();
    assert!(
        deleted.status.success(),
        "{}",
        String::from_utf8_lossy(&deleted.stdout)
    );
    let resumed = resume.wait_with_output().unwrap();
    assert!(!resumed.status.success());
    let response: Value = serde_json::from_slice(&resumed.stdout).unwrap();
    assert_eq!(response["error"]["code"], "session-not-found");
    assert!(!fixture.session.exists());
    assert!(
        !fixture.killed.exists(),
        "conditional delete must not signal"
    );
}

#[test]
fn stopped_only_delete_preserves_pending_termination_recovery() {
    let mut fixture = Fixture::new();
    let incarnation = fixture.mark_runtime_stopped();
    let record_path = fixture.session.join("session.json");
    let mut record: Value = serde_json::from_slice(&fs::read(&record_path).unwrap()).unwrap();
    record["delete_tmux_termination_state"] = json!({
        "launch_id": incarnation,
        "state": "pending",
        "thaw_on_recovery": true,
    });
    fs::write(&record_path, serde_json::to_vec(&record).unwrap()).unwrap();
    fs::write(&fixture.name_status, "stopped").unwrap();

    let output = fixture.delete_if_stopped(&incarnation);
    assert_ne!(output.code, 0, "{}", output.stdout_text());
    assert_eq!(
        output.stdout_json()["error"]["code"],
        "session-termination-in-progress"
    );
    assert_eq!(output.stdout_json()["error"]["details"]["retryable"], true);
    assert!(
        !fixture.killed.exists(),
        "conditional delete must not signal"
    );
    assert!(fixture.session.exists(), "recovery state must be retained");
}

#[test]
fn delete_recovery_rejects_a_replacement_tmux_target_without_killing_it() {
    let mut fixture = Fixture::new();
    for (field, replacement) in [("session_id", "$90"), ("pane_id", "%90")] {
        let mut record = fixture.record.clone();
        record["delete_tmux_identity"] = json!({
            "launch_id": record["runtime"]["launch_id"],
            "session_id": "$91", "pane_id": "%91", "pane_pid": fixture.pane.0.id(),
        });
        record["delete_tmux_identity"][field] = json!(replacement);
        let bytes = serde_json::to_vec(&record).unwrap();
        fs::write(fixture.session.join("session.json"), &bytes).unwrap();
        let output = fixture.delete();
        assert_eq!(output.code, 1, "{field}: {}", output.stdout_text());
        assert_eq!(
            output.stdout_json()["error"]["details"]["reason"],
            "runtime-identity-mismatch"
        );
        assert_eq!(
            fs::read(fixture.session.join("session.json")).unwrap(),
            bytes
        );
        assert!(
            !fixture.killed.exists(),
            "replacement target must not be signaled"
        );
        assert!(fixture.pane.0.try_wait().unwrap().is_none());
    }
}

#[test]
fn delete_recovery_retains_state_when_the_managed_name_is_not_stopped() {
    let mut fixture = Fixture::new();
    let before = fs::read(fixture.session.join("session.json")).unwrap();
    for status in ["running", "unknown"] {
        fs::write(&fixture.name_status, status).unwrap();
        let output = fixture.delete();
        assert_eq!(output.code, 1, "{status}: {}", output.stdout_text());
        assert_eq!(
            output.stdout_json()["error"]["details"]["reason"],
            "verification-failed"
        );
        assert!(
            fixture.killed.exists(),
            "fixture must acknowledge the conditional kill"
        );
        assert_eq!(
            fs::read(fixture.session.join("session.json")).unwrap(),
            before
        );
        assert!(fixture.pane.0.try_wait().unwrap().is_none());
        fs::remove_file(&fixture.killed).unwrap();
    }
    fs::write(&fixture.name_status, "stopped").unwrap();
    let output = fixture.delete();
    assert_eq!(output.code, 0, "{}", output.stdout_text());
    assert_eq!(output.stdout_json()["data"]["killed"], false);
    assert!(!fixture.session.exists());
    assert!(fixture.pane.0.try_wait().unwrap().is_none());
}
