//! Isolated CLI regressions for failed Codex account bindings.
use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};

use nils_test_support::cmd::{CmdOptions, CmdOutput, run_resolved};
use pretty_assertions::{assert_eq, assert_ne};
use serde_json::{Value, json};

struct Fixture {
    temp: tempfile::TempDir,
    state: PathBuf,
    record: PathBuf,
    tmux: PathBuf,
}

fn executable(path: &Path, body: &str) {
    fs::write(path, body).unwrap();
    fs::set_permissions(path, fs::Permissions::from_mode(0o700)).unwrap();
    assert!(
        std::process::Command::new(path)
            .arg("prewarm")
            .status()
            .unwrap()
            .success()
    );
}

impl Fixture {
    fn new() -> Self {
        let temp = tempfile::tempdir().unwrap();
        let state = temp.path().join("state");
        let dir = state.join("sessions/fixture");
        fs::create_dir_all(&dir).unwrap();
        let record = dir.join("session.json");
        let tmux = temp.path().join("tmux");
        executable(
            &tmux,
            r#"#!/bin/sh
[ "$1" = prewarm ] && exit 0
printf '%s\n' "$1" >> "$(dirname "$0")/tmux.calls"
exit 0
"#,
        );
        let fixture = Self {
            temp,
            state,
            record,
            tmux,
        };
        fixture.binding("failed");
        fixture
    }

    fn binding(&self, state: &str) {
        fs::write(
            &self.record,
            serde_json::to_vec_pretty(&json!({
                "schema_version": "agent-session.session.v1", "id": "fixture",
                "agent": "codex", "mode": "interactive", "cwd": self.temp.path(),
                "tmux_session": "modeled-fixture", "created_at": "2030-01-01T00:00:00Z",
                "updated_at": "2030-01-01T00:00:00Z",
                "runtime": {
                    "kind": "codex_app_server", "tmux_session": "modeled-fixture", "generation": 1,
                    "started_at": "2030-01-01T00:00:00Z", "launch_id": "runtime-fixture",
                    "codex_app_server_protocol": "v2",
                    "codex_app_server_socket": self.temp.path().join("control.sock"),
                    "codex_app_server_proxy": self.temp.path().join("proxy.sock"),
                    "codex_app_server_thread_handoff": self.temp.path().join("thread"),
                    "codex_app_server_thread_attached": self.temp.path().join("attached")
                },
                "codex_account_binding": {
                    "schema_version": "agent-session.codex-account-binding.v1",
                    "selected_account": "alpha", "revision": 7, "state": state,
                    "applied_runtime_id": "runtime-fixture", "updated_at": "2030-01-01T00:00:00Z"
                }
            }))
            .unwrap(),
        )
        .unwrap();
        fs::write(self.temp.path().join("tmux.calls"), "").unwrap();
    }

    fn options(&self) -> CmdOptions {
        CmdOptions::new()
            .with_cwd(self.temp.path())
            .with_env_remove_prefix("AGENT_SESSION_")
            .with_env(
                "AGENT_HOME",
                &self.temp.path().join("agent-home").to_string_lossy(),
            )
            .with_env("AGENT_SESSION_CODEX_BIN", "/nonexistent/fixture-codex")
            .with_env("AGENT_SESSION_CLAUDE_BIN", "/nonexistent/fixture-claude")
    }

    fn send(&self, args: &[&str]) -> CmdOutput {
        let mut argv = vec![
            "--state-dir",
            self.state.to_str().unwrap(),
            "send",
            "fixture",
            "--tmux-bin",
            self.tmux.to_str().unwrap(),
            "--format",
            "json",
        ];
        argv.extend_from_slice(args);
        run_resolved("agent-session", &argv, &self.options())
    }

    fn broker(&self, name: &str, account: &str) -> String {
        let path = self.temp.path().join(name);
        executable(
            &path,
            &format!(
                r#"#!/bin/sh
[ "$1" = prewarm ] && exit 0
[ "$1" = list ] || exit 2
printf '%s\n' '{{"schema_version":"agent-session.codex-auth-broker.v1","accounts":[{{"account":"{account}"}}]}}'
"#
            ),
        );
        serde_json::to_string(&[path]).unwrap()
    }

    fn switch(&self, account: &str, caller_broker: &str) -> CmdOutput {
        run_resolved(
            "agent-session",
            &[
                "--state-dir",
                self.state.to_str().unwrap(),
                "account",
                "switch",
                "fixture",
                "--account",
                account,
                "--expected-incarnation",
                "runtime-fixture",
                "--format",
                "json",
            ],
            &self
                .options()
                .with_env("AGENT_SESSION_CODEX_ACCOUNT_BROKER", caller_broker),
        )
    }
}

#[test]
fn literal_send_refuses_non_bound_binding_before_paste_or_keys() {
    let fixture = Fixture::new();
    for state in ["failed", "pending"] {
        for args in [
            vec!["--text", "hello"],
            vec!["--text", "hello", "--key", "escape", "--allow-blocked"],
        ] {
            fixture.binding(state);
            let before = fs::read(&fixture.record).unwrap();
            let output = fixture.send(&args);
            assert_eq!(
                output.stdout_json()["error"]["code"],
                "codex-account-not-bound",
                "state={state}, output={}",
                output.stdout_text()
            );
            assert_ne!(output.code, 0);
            let calls = fs::read_to_string(fixture.temp.path().join("tmux.calls")).unwrap();
            assert!(
                !calls
                    .lines()
                    .any(|call| matches!(call, "load-buffer" | "paste-buffer" | "send-keys")),
                "{calls}"
            );
            assert_eq!(fs::read(&fixture.record).unwrap(), before);
        }
    }
    fixture.binding("bound");
    let output = fixture.send(&["--text", "hello", "--key", "escape"]);
    assert_eq!(
        output.code,
        0,
        "{} {}",
        output.stdout_text(),
        output.stderr_text()
    );
    let calls = fs::read_to_string(fixture.temp.path().join("tmux.calls")).unwrap();
    assert!(calls.contains("paste-buffer"), "{calls}");
    assert!(calls.contains("send-keys"), "{calls}");
    // Non-submitting control keys remain available to answer a dialog.
    fixture.binding("failed");
    assert_eq!(fixture.send(&["--key", "escape"]).code, 0);
}

#[test]
fn repair_uses_serve_roster_before_binding_or_intent_mutation() {
    let fixture = Fixture::new();
    let serve_broker = fixture.broker("serve-broker", "alpha");
    let caller_broker = fixture.broker("caller-broker", "beta");
    let broker_record = fixture.state.join("serve/account-brokers.json");
    fs::create_dir_all(broker_record.parent().unwrap()).unwrap();
    fs::write(&broker_record, serde_json::to_vec(&json!({
        "schema_version": "agent-session.serve-account-brokers.v1",
        "brokers": {"AGENT_SESSION_CODEX_ACCOUNT_BROKER": serde_json::from_str::<Value>(&serve_broker).unwrap()}
    })).unwrap()).unwrap();
    fs::set_permissions(&broker_record, fs::Permissions::from_mode(0o600)).unwrap();

    let output = fixture.switch("alpha", &caller_broker);
    assert_eq!(
        output.code,
        0,
        "{} {}",
        output.stdout_text(),
        output.stderr_text()
    );
    let queued: Value = serde_json::from_slice(&fs::read(&fixture.record).unwrap()).unwrap();
    assert_eq!(queued["codex_account_binding"]["state"], "failed");
    assert_eq!(queued["codex_account_binding"]["revision"], 7);
    assert_eq!(queued["codex_account_next"]["account"], "alpha");
    assert_eq!(queued["codex_account_next"]["state"], "queued");

    let before = fs::read(&fixture.record).unwrap();
    let output = fixture.switch("beta", &caller_broker);
    assert_ne!(output.code, 0);
    assert_eq!(
        output.stdout_json()["error"]["code"],
        "codex-account-unknown"
    );
    assert_eq!(
        fs::read(&fixture.record).unwrap(),
        before,
        "a refused repair preserves the existing intent"
    );
}
