use crate::cli::{fake_agent, run, write_resumable_session_record_with_agent_bin};
use pretty_assertions::assert_eq;
use serde_json::{Value, json};
use std::{
    fs,
    os::unix::fs::PermissionsExt,
    path::PathBuf,
    process::{Child, Command},
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
   [ -f {killed} ] || exit 0
   [ "$3" != '$91' ] || absent
   [ "$3" = '=hs-claude-guarded-delete' ] || exit 42
   case "$(cat {name_status})" in
     running) exit 0 ;; unknown) exit 42 ;; stopped) absent ;; *) exit 42 ;;
   esac ;;
 *) exit 42 ;;
esac
"#,
            pid = pane.0.id(),
            state = shell_words::quote(state.to_str().unwrap()),
            runtime = shell_words::quote(record["runtime"]["launch_id"].as_str().unwrap()),
            killed = shell_words::quote(killed.to_str().unwrap()),
            name_status = shell_words::quote(name_status.to_str().unwrap()),
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
