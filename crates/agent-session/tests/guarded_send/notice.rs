//! Notice delivery uses the existing guarded native transport without terminal fallback.
use super::native::Server;
use super::*;
use nils_test_support::cmd::{CmdOptions, CmdOutput, run_resolved};
use pretty_assertions::assert_eq;

impl Fixture {
    fn notice(&self, extra: &[&str]) -> CmdOutput {
        let body = self.root().join("notice-body");
        let state = self.root().join("state");
        let tmux = self.root().join("tmux");
        fs::write(&body, "  synthetic\r\n\n本文 <tag>\n").unwrap();
        let mut args = vec![
            "--state-dir",
            state.to_str().unwrap(),
            "notice",
            "guarded",
            "--kind",
            "schedule.tick",
            "--source",
            "scheduler",
            "--id",
            "Example-1",
            "--body-file",
            body.to_str().unwrap(),
            "--tmux-bin",
            tmux.to_str().unwrap(),
            "--format",
            "json",
            "--idempotency-key",
            "notice-once",
        ];
        args.extend_from_slice(extra);
        run_resolved(
            "agent-session",
            &args,
            &CmdOptions::new()
                .with_cwd(self.root())
                .with_env_remove_prefix("AGENT_SESSION_")
                .with_env("AGENT_SESSION_NOTICE_WRITERS", "1"),
        )
    }
}

#[test]
fn notice_native_preserves_exact_bytes_and_idempotency() {
    let fixture = Fixture::new("codex");
    let _capability = fixture.bind_codex();
    let server = Server::start(&fixture, "idle");
    let output = fixture.notice(&[]);
    assert_eq!(
        output.code,
        0,
        "{} {}",
        output.stdout_text(),
        output.stderr_text()
    );
    assert_eq!(
        output.stdout_json()["schema_version"],
        "cli.agent-session.notice.v1"
    );
    assert_eq!(
        server.mutations()[0]["params"]["input"][0]["text"],
        "<!-- agent-notice:v1 kind=schedule.tick source=scheduler id=Example-1 -->\n\n  synthetic\r\n\n本文 <tag>\n"
    );
    fixture.refused(&fixture.notice(&[]), "send-idempotency-key-used");
    assert_eq!(server.mutations().len(), 1);
    fixture.no_delivery();
}

#[test]
fn notice_native_guards_refuse_without_input() {
    for (mode, draft, extra, code) in [
        ("busy", "", vec![], "send-session-not-idle"),
        ("idle", "human draft", vec![], "send-composer-not-empty"),
        (
            "idle",
            "",
            vec!["--expect-runtime", "wrong"],
            "send-identity-mismatch",
        ),
        (
            "idle",
            "",
            vec!["--expect-provider-session", "wrong"],
            "send-identity-mismatch",
        ),
    ] {
        let fixture = Fixture::new("codex");
        let _capability = fixture.bind_codex();
        let server = Server::start(&fixture, mode);
        fs::write(fixture.root().join("composer"), draft).unwrap();
        fixture.refused(&fixture.notice(&extra), code);
        assert!(server.mutations().is_empty());
        fixture.no_delivery();
    }
}

#[test]
fn notice_native_unknown_outcome_never_replays() {
    let fixture = Fixture::new("codex");
    let _capability = fixture.bind_codex();
    let server = Server::start(&fixture, "disconnect");
    let value = fixture.refused(&fixture.notice(&[]), "send-native-unconfirmed");
    assert_eq!(value["error"]["details"]["submitted"], Value::Null);
    fixture.refused(&fixture.notice(&[]), "send-idempotency-key-used");
    assert_eq!(server.mutations().len(), 1);
    fixture.no_delivery();
}

#[test]
fn notice_terminal_refuses_before_any_input() {
    for provider in ["claude", "codex"] {
        let fixture = Fixture::new(provider);
        fixture.refused(&fixture.notice(&[]), "notice-transport-unsupported");
        fixture.no_delivery();
    }
}
