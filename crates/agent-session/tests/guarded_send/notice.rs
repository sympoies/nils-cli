//! Notice delivery uses the existing guarded native transport without terminal fallback.
use super::native::Server;
use super::*;
use nils_test_support::cmd::{CmdOptions, CmdOutput, run_resolved};
use pretty_assertions::assert_eq;

impl Fixture {
    fn notice(&self, extra: &[&str]) -> CmdOutput {
        fs::write(self.root().join("notice-mode"), "").unwrap();
        let body = self.root().join("notice-body");
        let state = self.root().join("state");
        let tmux = self.root().join("tmux");
        if !body.exists() {
            fs::write(&body, "  synthetic\r\n\n本文 <tag>\n").unwrap();
        }
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
fn notice_terminal_delivers_the_envelope_through_guarded_input() {
    for provider in ["claude", "codex"] {
        let fixture = Fixture::new(provider);
        fs::write(fixture.root().join("notice-body"), "synthetic").unwrap();
        fs::write(fixture.root().join("start-turn-on-enter"), "").unwrap();
        let output = fixture.notice(&[]);
        assert_eq!(
            output.code,
            0,
            "{} calls={:?}",
            output.stdout_text(),
            fixture.calls()
        );
        assert_eq!(
            output.stdout_json()["schema_version"],
            "cli.agent-session.notice.v1"
        );
        let envelope = "<!-- agent-notice:v1 kind=schedule.tick source=scheduler id=Example-1 -->\n\nsynthetic";
        let submissions: Vec<String> = fs::read_to_string(fixture.root().join("submissions"))
            .unwrap()
            .lines()
            .map(|line| serde_json::from_str(line).unwrap())
            .collect();
        assert_eq!(submissions, [envelope]);
        let calls = fixture.calls();
        assert!(calls.iter().any(|call| call[0] == "paste-buffer"));
        assert!(calls.iter().any(|call| call[0] == "send-keys" && call.last().is_some_and(|key| key == "Enter")));
    }
}

#[test]
fn claude_terminal_notice_soft_wrapped_body_submits_exactly_once() {
    let fixture = Fixture::new("claude");
    let body = "x".repeat(100);
    fs::write(fixture.root().join("notice-body"), &body).unwrap();
    fs::write(fixture.root().join("start-turn-on-enter"), "").unwrap();
    fs::write(fixture.root().join("wrap-80"), "").unwrap();

    let output = fixture.notice(&[]);
    assert_eq!(output.code, 0, "{}", output.stdout_text());
    let envelope = format!(
        "<!-- agent-notice:v1 kind=schedule.tick source=scheduler id=Example-1 -->\n\n{body}"
    );
    let submissions: Vec<String> = fs::read_to_string(fixture.root().join("submissions"))
        .unwrap()
        .lines()
        .map(|line| serde_json::from_str(line).unwrap())
        .collect();
    assert_eq!(submissions, [envelope]);
    assert!(
        fixture
            .calls()
            .iter()
            .any(|call| { call[0] == "capture-pane" && call.iter().any(|arg| arg == "-J") })
    );
}

#[test]
fn claude_terminal_notice_unicode_body_uses_terminal_cell_width() {
    let fixture = Fixture::new("claude");
    fs::write(fixture.root().join("notice-body"), "本文").unwrap();
    fs::write(fixture.root().join("start-turn-on-enter"), "").unwrap();

    let output = fixture.notice(&[]);
    assert_eq!(output.code, 0, "{}", output.stdout_text());
    let submissions: Vec<String> = fs::read_to_string(fixture.root().join("submissions"))
        .unwrap()
        .lines()
        .map(|line| serde_json::from_str(line).unwrap())
        .collect();
    assert_eq!(
        submissions,
        ["<!-- agent-notice:v1 kind=schedule.tick source=scheduler id=Example-1 -->\n\n本文"]
    );
}

#[test]
fn notice_terminal_refusal_leaves_existing_composer_untouched() {
    for provider in ["claude", "codex"] {
        let fixture = Fixture::new(provider);
        fs::write(fixture.root().join("composer"), "existing draft").unwrap();
        fixture.refused(&fixture.notice(&[]), "send-composer-not-empty");
        assert_eq!(
            fs::read_to_string(fixture.root().join("composer")).unwrap(),
            "existing draft"
        );
        fixture.no_delivery();
    }
}

#[test]
fn notice_terminal_hard_newline_edit_is_not_submitted_as_a_soft_wrap() {
    for provider in ["claude", "codex"] {
        let fixture = Fixture::new(provider);
        fs::write(fixture.root().join("notice-body"), "A B").unwrap();
        fs::write(fixture.root().join("start-turn-on-enter"), "").unwrap();
        fs::write(
            fixture.root().join("paste-replacement"),
            "<!-- agent-notice:v1 kind=schedule.tick source=scheduler id=Example-1 -->\n\nA\nB",
        )
        .unwrap();
        let output = fixture.notice(&[]);
        fixture.refused(&output, "send-composer-mismatch");
        assert!(fixture.calls().iter().all(|call| {
            !(call[0] == "send-keys" && call.last().is_some_and(|key| key == "Enter"))
        }));
        assert!(!fixture.root().join("submissions").exists());
        fixture.refused(&fixture.notice(&[]), "send-idempotency-key-used");
    }
}

#[test]
fn claude_terminal_notice_row_ending_spaces_refuse_before_claim() {
    let fixture = Fixture::new("claude");
    fs::write(fixture.root().join("notice-body"), "A \nB").unwrap();
    let value = fixture.refused(&fixture.notice(&[]), "send-notice-trailing-spaces");
    assert_eq!(value["error"]["details"]["stage"], "admission");
    fixture.no_delivery();

    fs::write(fixture.root().join("start-turn-on-enter"), "").unwrap();
    fs::write(fixture.root().join("notice-body"), "A\nB").unwrap();
    let retry = fixture.notice(&[]);
    assert_eq!(retry.code, 0, "{}", retry.stdout_text());
}

#[test]
fn claude_terminal_notice_crlf_row_ending_spaces_refuse_before_claim() {
    let fixture = Fixture::new("claude");
    fs::write(fixture.root().join("notice-body"), "A \r\nB").unwrap();
    let value = fixture.refused(&fixture.notice(&[]), "send-notice-trailing-spaces");
    assert_eq!(value["error"]["details"]["stage"], "admission");
    fixture.no_delivery();
}

#[test]
fn claude_terminal_notice_final_row_spaces_refuse_before_claim() {
    let fixture = Fixture::new("claude");
    fs::write(fixture.root().join("notice-body"), "A ").unwrap();
    let value = fixture.refused(&fixture.notice(&[]), "send-notice-trailing-spaces");
    assert_eq!(value["error"]["details"]["stage"], "admission");
    fixture.no_delivery();
}

#[test]
fn claude_terminal_notice_lone_cr_row_ending_spaces_refuse_before_claim() {
    let fixture = Fixture::new("claude");
    fs::write(fixture.root().join("notice-body"), "A \rB").unwrap();
    let value = fixture.refused(&fixture.notice(&[]), "send-notice-trailing-spaces");
    assert_eq!(value["error"]["details"]["stage"], "admission");
    fixture.no_delivery();
}

#[test]
fn claude_terminal_notice_extra_trailing_space_is_not_renderer_padding() {
    let fixture = Fixture::new("claude");
    fs::write(fixture.root().join("notice-body"), "A B").unwrap();
    fs::write(fixture.root().join("start-turn-on-enter"), "").unwrap();
    fs::write(fixture.root().join("live-layout"), "").unwrap();
    fs::write(
        fixture.root().join("paste-replacement"),
        "<!-- agent-notice:v1 kind=schedule.tick source=scheduler id=Example-1 -->\n\nA B ",
    )
    .unwrap();
    let output = fixture.notice(&[]);
    assert_eq!(output.code, 1, "{}", output.stderr_text());
    let refusal = output.stdout_json();
    assert!(matches!(
        refusal["error"]["code"].as_str(),
        Some("send-composer-unobservable" | "send-guard-observation-timeout")
    ));
    assert!(fixture.calls().iter().all(|call| {
        !(call[0] == "send-keys" && call.last().is_some_and(|key| key == "Enter"))
    }));
    assert!(!fixture.root().join("submissions").exists());
    fixture.refused(&fixture.notice(&[]), "send-idempotency-key-used");
}

#[test]
fn queued_terminal_notice_waits_for_idle_and_submits_once() {
    let fixture = Fixture::new("claude");
    fs::write(fixture.root().join("notice-body"), "synthetic").unwrap();
    fs::write(fixture.root().join("start-turn-on-enter"), "").unwrap();
    let activity = fixture.root().join("state/sessions/guarded/activity.json");
    let mut state: Value = serde_json::from_slice(&fs::read(&activity).unwrap()).unwrap();
    state["state"]["phase"] = Value::String("working".into());
    fs::write(&activity, serde_json::to_vec(&state).unwrap()).unwrap();
    let idle_activity = activity.clone();
    let release = std::thread::spawn(move || {
        std::thread::sleep(std::time::Duration::from_millis(600));
        let mut state: Value = serde_json::from_slice(&fs::read(&idle_activity).unwrap()).unwrap();
        state["state"]["phase"] = Value::String("waiting".into());
        fs::write(&idle_activity, serde_json::to_vec(&state).unwrap()).unwrap();
    });

    let output = fixture.notice(&["--queue"]);
    release.join().unwrap();
    assert_eq!(output.code, 0, "{}", output.stdout_text());
    let submissions: Vec<String> = fs::read_to_string(fixture.root().join("submissions"))
        .unwrap()
        .lines()
        .map(|line| serde_json::from_str(line).unwrap())
        .collect();
    assert_eq!(
        submissions,
        ["<!-- agent-notice:v1 kind=schedule.tick source=scheduler id=Example-1 -->\n\nsynthetic"]
    );
    fixture.idle();
    fixture.refused(&fixture.notice(&["--queue"]), "send-idempotency-key-used");
    assert_eq!(
        fs::read_to_string(fixture.root().join("submissions"))
            .unwrap()
            .lines()
            .count(),
        1
    );
}
