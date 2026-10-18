use super::cli::{
    fake_agent, fake_tmux, run, tmux_calls, write_resumable_session_record_with_agent_bin,
};
use nils_test_support::cmd::CmdOutput;
use pretty_assertions::{assert_eq, assert_ne};
use serde_json::{Value, json};
use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::PathBuf;
use std::time::{Duration, Instant};

struct Fixture {
    tmp: tempfile::TempDir,
    state: PathBuf,
    tmux: PathBuf,
    log: PathBuf,
    record: PathBuf,
}

impl Fixture {
    fn new(agent: &str) -> Self {
        let tmp = tempfile::tempdir().unwrap();
        let state = tmp.path().join("state");
        let cwd = tmp.path().join("repo");
        fs::create_dir(&cwd).unwrap();
        let (tmux, log) = fake_tmux(tmp.path());
        let backend = tmp.path().join("tmux-backend");
        fs::rename(&tmux, &backend).unwrap();
        // Inject a transient transport failure only after the test arms it.
        // All subsequent probes use the fixture's real live/stopped state.
        fs::write(
            &tmux,
            format!(
                r#"#!/bin/sh
if [ "$1" = has-session ] && [ -f "$AGENT_SESSION_FAKE_TMUX_LOG.permanent" ]; then
  case "$(cat "$AGENT_SESSION_FAKE_TMUX_LOG.permanent")" in
    exit-127) exit 127 ;;
    permission-denied) printf '%s\n' 'permission denied opening fixture socket' >&2; exit 1 ;;
  esac
fi
if [ "$1" = has-session ] && [ -f "$AGENT_SESSION_FAKE_TMUX_LOG.hung" ]; then
  sleep 30
  exit 42
fi
if [ "$1" = has-session ] && [ -f "$AGENT_SESSION_FAKE_TMUX_LOG.uncertain" ]; then
  if [ "${{RESUME_STATUS_PERSISTENT:-0}}" != 1 ]; then
    rm -f "$AGENT_SESSION_FAKE_TMUX_LOG.uncertain"
  fi
  printf '%s\n' 'server exited unexpectedly' >&2
  exit 1
fi
if [ "$1" = new-session ]; then
  rm -f "$AGENT_SESSION_FAKE_TMUX_LOG.killed"
fi
exec {} "$@"
"#,
                shell_words::quote(&backend.to_string_lossy()),
            ),
        )
        .unwrap();
        fs::set_permissions(&tmux, fs::Permissions::from_mode(0o700)).unwrap();
        let provider = fake_agent(tmp.path(), agent);
        let args = if agent == "codex" {
            vec![
                "resume",
                "resume-session-id",
                "--cd",
                cwd.to_str().unwrap(),
                "--no-alt-screen",
            ]
        } else {
            vec!["--resume", "resume-session-id"]
        };
        let session = write_resumable_session_record_with_agent_bin(
            &state,
            "resume-fixture",
            agent,
            "hs-resume-fixture",
            &cwd,
            &args,
            Some(&provider),
        );
        let fixture = Self {
            tmp,
            state,
            tmux,
            log,
            record: session.join("session.json"),
        };
        let first = fixture.resume(false);
        assert_eq!(first.code, 0, "{}", first.stdout_text());
        fixture
    }

    fn resume(&self, persistent: bool) -> CmdOutput {
        run(
            self.tmp.path(),
            &[
                "--state-dir",
                self.state.to_str().unwrap(),
                "resume",
                "resume-fixture",
                "--tmux-bin",
                self.tmux.to_str().unwrap(),
                "--format",
                "json",
            ],
            &[
                ("AGENT_SESSION_FAKE_TMUX_LOG", self.log.to_str().unwrap()),
                ("AGENT_SESSION_FAKE_TMUX_ABSENT_BEFORE_LAUNCH", "1"),
                ("AGENT_SESSION_FAKE_TMUX_ABSENT_AFTER_KILL", "1"),
                (
                    "RESUME_STATUS_PERSISTENT",
                    if persistent { "1" } else { "0" },
                ),
            ],
        )
    }

    fn arm_unknown(&self) {
        fs::write(self.log.with_extension("log.uncertain"), b"uncertain").unwrap();
    }

    fn kill(&self) {
        let group = self.stored()["delete_tmux_identity"]["process_group_id"]
            .as_i64()
            .unwrap()
            .to_string();
        let status = std::process::Command::new(&self.tmux)
            .args(["kill-session", "-t", "hs-resume-fixture"])
            .env("AGENT_SESSION_FAKE_TMUX_LOG", &self.log)
            .env("AGENT_SESSION_FAKE_TMUX_PROCESS_GROUP_ID", group)
            .status()
            .unwrap();
        assert!(status.success());
    }

    fn launches(&self) -> usize {
        tmux_calls(&self.log)
            .iter()
            .filter(|call| call.first().is_some_and(|arg| arg == "new-session"))
            .count()
    }

    fn stored(&self) -> Value {
        serde_json::from_slice(&fs::read(&self.record).unwrap()).unwrap()
    }
}

#[test]
fn resume_status_recovers_immediately_after_runtime_kill() {
    for agent in ["codex", "claude"] {
        let fixture = Fixture::new(agent);
        let before = fixture.stored();
        fixture.kill();
        fixture.arm_unknown();
        // Immediately resume: no heartbeat-expiry wait or caller retry.
        let resumed = fixture.resume(false);
        assert_eq!(resumed.code, 0, "{}", resumed.stdout_text());
        let after = fixture.stored();
        assert_eq!(after["id"], before["id"]);
        assert_eq!(after["tmux_session"], before["tmux_session"]);
        assert_eq!(after["provider_resume"], before["provider_resume"]);
        assert_eq!(after["runtime"]["generation"], json!(3));
        assert_ne!(
            after["runtime"]["launch_id"],
            before["runtime"]["launch_id"]
        );
        assert_eq!(fixture.launches(), 2);
        let again = fixture.resume(false);
        assert_eq!(again.code, 0, "{}", again.stdout_text());
        assert_eq!(fixture.launches(), 2, "resume must not launch twice");
    }
}

#[test]
fn resume_status_transient_unknown_then_running_is_idempotent() {
    let fixture = Fixture::new("claude");
    let before = fixture.stored();
    fixture.arm_unknown();
    let resumed = fixture.resume(false);
    assert_eq!(resumed.code, 0, "{}", resumed.stdout_text());
    assert_eq!(fixture.stored(), before);
    assert_eq!(fixture.launches(), 1);
}

#[test]
fn resume_status_persistent_unknown_is_bounded_and_retryable() {
    let fixture = Fixture::new("claude");
    fixture.kill();
    let before = fixture.stored();
    fixture.arm_unknown();
    let started = Instant::now();
    let resumed = fixture.resume(true);
    assert!(started.elapsed() < Duration::from_secs(8));
    assert_eq!(resumed.code, 1);
    let error = &resumed.stdout_json()["error"];
    assert_eq!(error["code"], "session-status-unknown");
    assert_eq!(error["details"]["retryable"], true);
    assert_eq!(error["details"]["next_action"], "retry-resume");
    assert_eq!(fixture.stored(), before);
    assert_eq!(fixture.launches(), 1, "unknown status must never launch");
}

#[test]
fn resume_status_hung_probe_respects_total_budget() {
    let fixture = Fixture::new("claude");
    fixture.kill();
    let before = fixture.stored();
    fs::write(fixture.log.with_extension("log.hung"), b"hung").unwrap();
    let started = Instant::now();
    let resumed = fixture.resume(false);
    assert!(started.elapsed() < Duration::from_secs(8));
    assert_eq!(resumed.code, 1);
    assert_eq!(
        resumed.stdout_json()["error"]["code"],
        "session-status-unknown"
    );
    assert_eq!(fixture.stored(), before);
    assert_eq!(fixture.launches(), 1);
}

#[test]
fn resume_status_concurrent_requests_launch_one_replacement() {
    let fixture = Fixture::new("claude");
    fixture.kill();
    fixture.arm_unknown();
    std::thread::scope(|scope| {
        let first = scope.spawn(|| fixture.resume(false));
        let second = scope.spawn(|| fixture.resume(false));
        let results = [first.join().unwrap(), second.join().unwrap()];
        assert!(results.iter().any(|resumed| resumed.code == 0));
        for resumed in results {
            if resumed.code != 0 {
                // A waiter that observed the old incarnation before taking the
                // lock is refused after the first caller replaces it.
                assert_eq!(
                    resumed.stdout_json()["error"]["code"],
                    "session-runtime-changed"
                );
            }
        }
    });
    assert_eq!(fixture.launches(), 2);
    assert_eq!(fixture.stored()["runtime"]["generation"], 3);
}

#[test]
fn resume_status_permanent_probe_failure_is_not_retryable() {
    for cause in ["exit-127", "permission-denied", "missing-executable"] {
        let fixture = Fixture::new("claude");
        fixture.kill();
        let before = fixture.stored();
        if cause == "missing-executable" {
            fs::rename(&fixture.tmux, fixture.tmp.path().join("disabled-tmux")).unwrap();
        } else {
            fs::write(fixture.log.with_extension("log.permanent"), cause).unwrap();
        }
        let resumed = fixture.resume(false);
        assert_eq!(resumed.code, 1, "{cause}");
        let error = &resumed.stdout_json()["error"];
        assert_eq!(error["code"], "session-status-unknown", "{cause}");
        assert_eq!(error["details"]["retryable"], false, "{cause}");
        assert_eq!(error["details"]["next_action"], "inspect-tmux", "{cause}");
        assert_eq!(fixture.stored(), before);
        assert_eq!(fixture.launches(), 1);
    }
}
