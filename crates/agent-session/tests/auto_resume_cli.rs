use std::{fs, os::unix::fs::PermissionsExt};

use nils_test_support::cmd::{CmdOptions, CmdOutput, run_resolved};
use pretty_assertions::assert_eq;
use serde_json::{Value, json};

struct Fixture {
    tmp: tempfile::TempDir,
}

impl Fixture {
    fn new(agent: &str) -> Self {
        let tmp = tempfile::tempdir().unwrap();
        let dir = tmp.path().join("sessions/local-session");
        fs::create_dir_all(&dir).unwrap();
        fs::set_permissions(&dir, fs::Permissions::from_mode(0o700)).unwrap();
        fs::write(
            dir.join("session.json"),
            serde_json::to_vec(&json!({
                "schema_version": "agent-session.session.v1",
                "id": "local-session", "agent": agent, "mode": "interactive",
                "cwd": tmp.path(), "tmux_session": "fixture-session",
                "created_at": "2030-01-01T00:00:00Z",
                "updated_at": "2030-01-01T00:00:00Z",
                "runtime": {
                    "schema_version": "agent-session.runtime.v1",
                    "kind": "codex_app_server", "generation": 1,
                    "tmux_session": "fixture-session", "started_at": "2030-01-01T00:00:00Z",
                    "launch_id": "fixture-runtime",
                    "codex_app_server_protocol": "v2",
                    "codex_app_server_socket": tmp.path().join("codex.sock"),
                    "codex_app_server_proxy": tmp.path().join("codex.proxy"),
                    "codex_app_server_thread_handoff": tmp.path().join("codex.thread"),
                    "codex_app_server_thread_attached": tmp.path().join("codex.attached")
                }
            }))
            .unwrap(),
        )
        .unwrap();
        let tmux = tmp.path().join("tmux");
        fs::write(&tmux, "#!/bin/sh\nexit 1\n").unwrap();
        fs::set_permissions(&tmux, fs::Permissions::from_mode(0o755)).unwrap();
        Self { tmp }
    }

    fn run(&self, args: &[&str]) -> CmdOutput {
        let mut argv = vec!["--state-dir", self.tmp.path().to_str().unwrap()];
        argv.extend_from_slice(args);
        run_resolved(
            "agent-session",
            &argv,
            &CmdOptions::new()
                .with_cwd(self.tmp.path())
                .without_ambient_managed_session_env()
                .with_env_remove_many(&[
                    "AGENT_SESSION_SERVE_TOKEN",
                    "AGENT_SESSION_ACCOUNT_BROKER",
                    "AGENT_SESSION_CODEX_ACCOUNT_BROKER",
                ])
                .with_env(
                    "AGENT_SESSION_TMUX_BIN",
                    self.tmp.path().join("tmux").to_str().unwrap(),
                ),
        )
    }

    fn auto_resume(&self, flags: &[&str]) -> Value {
        let mut args = vec!["auto-resume", "local-session", "--format", "json"];
        args.extend_from_slice(flags);
        let output = self.run(&args);
        assert_eq!(output.code, 0, "{}", output.stdout_text());
        let body = output.stdout_json();
        assert_eq!(body["schema_version"], "cli.agent-session.auto-resume.v1");
        assert_eq!(body["ok"], true);
        body["data"].clone()
    }
}

#[test]
fn local_auto_resume_opt_in_is_durable_and_visible_without_a_token() {
    let fixture = Fixture::new("codex");
    assert_eq!(fixture.auto_resume(&[])["enabled"], false);
    assert!(
        !fixture
            .tmp
            .path()
            .join("sessions/local-session/auto-resume.json")
            .exists(),
        "status must not create durable state"
    );
    let enabled = fixture.auto_resume(&["--enable", "--policy", "wait_for_reset"]);
    assert_eq!(enabled["supported"], true);
    assert_eq!(enabled["enabled"], true);
    assert_eq!(enabled["recovery_policy"], "wait_for_reset");
    assert_eq!(fixture.auto_resume(&[]), enabled);
    let listed = fixture.run(&["list", "--format", "json"]);
    assert_eq!(listed.code, 0, "{}", listed.stdout_text());
    assert_eq!(
        listed.stdout_json()["data"][0]["auto_resume"]["enabled"],
        true
    );
    assert_eq!(fixture.auto_resume(&["--disable"])["enabled"], false);
    assert_eq!(fixture.auto_resume(&[])["enabled"], false);
}

#[test]
fn local_auto_resume_rejects_invalid_mutations_without_writing_state() {
    let fixture = Fixture::new("codex");
    for flags in [
        vec!["--enable", "--disable"],
        vec!["--policy", "wait_for_reset"],
        vec!["--disable", "--policy", "wait_for_reset"],
        vec!["--enable", "--policy", "invalid"],
    ] {
        let mut args = vec!["auto-resume", "local-session", "--format", "json"];
        args.extend_from_slice(&flags);
        let output = fixture.run(&args);
        assert_eq!(output.code, 64, "{}", output.stdout_text());
        let body = output.stdout_json();
        assert_eq!(body["schema_version"], "cli.agent-session.auto-resume.v1");
        assert_eq!(body["error"]["code"], "parse-error");
    }
    assert!(
        !fixture
            .tmp
            .path()
            .join("sessions/local-session/auto-resume.json")
            .exists()
    );
}

#[test]
fn local_auto_resume_keeps_shared_provider_and_failover_guards() {
    let fixture = Fixture::new("dsh");
    assert_eq!(fixture.auto_resume(&[])["supported"], false);
    let refused = fixture.run(&[
        "auto-resume",
        "local-session",
        "--enable",
        "--format",
        "json",
    ]);
    assert_eq!(
        refused.stdout_json()["error"]["code"],
        "auto-resume-unsupported"
    );
    assert_ne!(refused.code, 0);
    assert_eq!(fixture.auto_resume(&["--disable"])["enabled"], false);

    let codex = Fixture::new("codex");
    let refused = codex.run(&[
        "auto-resume",
        "local-session",
        "--enable",
        "--policy",
        "next_account_then_resume",
        "--format",
        "json",
    ]);
    assert_eq!(
        refused.stdout_json()["error"]["code"],
        "auto-resume-account-failover-unsupported"
    );
    assert_ne!(refused.code, 0);
    assert!(
        !codex
            .tmp
            .path()
            .join("sessions/local-session/auto-resume.json")
            .exists()
    );
}

#[test]
fn local_auto_resume_missing_session_and_text_status_are_readable() {
    let fixture = Fixture::new("codex");
    let missing = fixture.run(&["auto-resume", "missing-session", "--format", "json"]);
    assert_ne!(missing.code, 0);
    let body = missing.stdout_json();
    assert_eq!(body["schema_version"], "cli.agent-session.auto-resume.v1");
    assert_eq!(body["error"]["code"], "session-not-found");
    let enabled = fixture.run(&["auto-resume", "local-session", "--enable"]);
    assert_eq!(enabled.code, 0, "{}", enabled.stdout_text());
    assert!(enabled.stdout_text().contains("enabled: true"));
    assert!(enabled.stdout_text().contains("wait_for_reset"));
}
