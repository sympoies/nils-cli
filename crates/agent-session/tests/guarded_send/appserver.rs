//! The modeled proxy lease is held through the real CLI guarded-send path.
use super::*;
use pretty_assertions::assert_eq;

const GUARDS: &[&str] = &[
    "--require-idle",
    "--require-empty-composer",
    "--expect-runtime",
    "runtime-one",
    "--expect-provider-session",
    "provider-one",
    "--idempotency-key",
    "appserver-continuation",
];

#[test]
fn appserver_idle_empty_composer_submits_once_and_retry_refuses() {
    let fixture = Fixture::new("codex");
    let _capability = fixture.bind_codex();
    fixture.arm_recovery();
    let output = fixture.send(GUARDS);
    assert_eq!(output.code, 0, "{}", output.stdout_text());
    let value = output.stdout_json();
    assert_eq!(value["data"]["submission"]["outcome"], "submitted");
    assert_eq!(value["data"]["submission"]["enter_presses"], 1);
    assert_eq!(value["data"]["guards"]["atomic"], false);
    fixture.input_control_state();
    let calls = fixture.calls();
    assert_eq!(
        calls
            .iter()
            .filter(|call| call[0] == "paste-buffer")
            .count(),
        1
    );
    assert_eq!(
        calls.iter().filter(|call| call[0] == "send-keys").count(),
        1
    );
    assert!(calls.iter().any(|call| {
        call[0] == "capture-pane" && call.last().unwrap() == "modeled-guarded:0.0"
    }));
    let submissions = fs::read_to_string(fixture.root().join("submissions")).unwrap();
    let prompts: Vec<String> = submissions
        .lines()
        .map(|line| serde_json::from_str(line).unwrap())
        .collect();
    assert_eq!(prompts, ["continue-fixture"]);
    fixture.refused(&fixture.send(GUARDS), "send-idempotency-key-used");
    assert_eq!(
        fs::read_to_string(fixture.root().join("submissions")).unwrap(),
        submissions
    );
    assert_eq!(
        fixture
            .calls()
            .iter()
            .filter(|call| call[0] == "paste-buffer")
            .count(),
        1
    );
}

#[test]
fn appserver_drafts_refuse_without_paste_or_control_mutation() {
    for draft in ["user draft", " "] {
        let fixture = Fixture::new("codex");
        let _capability = fixture.bind_codex();
        let recovery = fixture.arm_recovery();
        fs::write(fixture.root().join("composer"), draft).unwrap();
        fixture.refused(&fixture.send(GUARDS), "send-composer-not-empty");
        fixture.unchanged_control_state(&recovery);
        assert_eq!(
            fs::read_to_string(fixture.root().join("composer")).unwrap(),
            draft
        );
    }
}

#[test]
fn appserver_turn_started_during_buffer_load_refuses_before_paste() {
    let fixture = Fixture::new("codex");
    let _capability = fixture.bind_codex();
    let recovery = fixture.arm_recovery();
    fs::write(fixture.root().join("start-on-load"), "").unwrap();
    fixture.refused(&fixture.send(GUARDS), "send-session-not-idle");
    assert!(fixture.calls().iter().any(|call| call[0] == "load-buffer"));
    fixture.unchanged_control_state(&recovery);
}

#[test]
fn appserver_post_paste_draft_refuses_enter_and_retains_key() {
    let fixture = Fixture::new("codex");
    let _capability = fixture.bind_codex();
    fs::write(fixture.root().join("inject-draft"), "").unwrap();
    let value = fixture.refused(&fixture.send(GUARDS), "send-composer-mismatch");
    assert_eq!(value["error"]["details"]["stage"], "submit");
    assert_eq!(value["error"]["details"]["pasted"], true);
    assert!(!fixture.root().join("submissions").exists());
    assert_eq!(
        fs::read_to_string(fixture.root().join("composer")).unwrap(),
        "continue-fixture user addition"
    );
    fixture.refused(&fixture.send(GUARDS), "send-idempotency-key-used");
    assert_eq!(
        fixture
            .calls()
            .iter()
            .filter(|call| call[0] == "paste-buffer")
            .count(),
        1
    );
    assert!(fixture.calls().iter().all(|call| call[0] != "send-keys"));
}

#[test]
fn appserver_runtime_identity_change_remains_refused_before_input() {
    let fixture = Fixture::new("codex");
    let _capability = fixture.bind_codex();
    let recovery = fixture.arm_recovery();
    fs::write(fixture.root().join("flip-second-runtime-read"), "").unwrap();
    fixture.refused(&fixture.send(GUARDS), "send-identity-mismatch");
    fixture.unchanged_control_state(&recovery);
}

#[test]
fn appserver_unknown_runtime_or_invalid_profile_remains_unsupported() {
    for (provider, kind, protocol) in [
        ("codex", "unknown", "v2"),
        ("codex", "codex_app_server", "unknown"),
        ("claude", "codex_app_server", "v2"),
    ] {
        let fixture = Fixture::new(provider);
        let _capability = fixture.bind_codex();
        let path = fixture.root().join("state/sessions/guarded/session.json");
        let mut record = fixture.document("session.json");
        record["runtime"]["kind"] = json!(kind);
        record["runtime"]["codex_app_server_protocol"] = json!(protocol);
        fs::write(path, serde_json::to_vec(&record).unwrap()).unwrap();
        let value = fixture.refused(&fixture.send(GUARDS), "send-guard-unsupported");
        assert_eq!(value["error"]["details"]["guard"], "provider");
        fixture.no_delivery();
    }
}
