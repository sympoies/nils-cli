//! The modeled proxy lease is held through the real CLI guarded-send path.
use super::native::Server;
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
    let server = Server::start(&fixture, "idle");
    fixture.arm_recovery();
    let output = fixture.send(GUARDS);
    assert_eq!(output.code, 0, "{}", output.stdout_text());
    let value = output.stdout_json();
    assert_eq!(value["data"]["submission"]["outcome"], "submitted");
    assert_eq!(value["data"]["submission"]["enter_presses"], 0);
    assert_eq!(value["data"]["guards"]["atomic"], false);
    fixture.input_control_state();
    fixture.no_delivery();
    assert_eq!(server.mutations().len(), 1);
    fixture.refused(&fixture.send(GUARDS), "send-idempotency-key-used");
    assert_eq!(server.mutations().len(), 1);
}

#[test]
fn appserver_drafts_refuse_without_paste_or_control_mutation() {
    for draft in ["user draft", " "] {
        let fixture = Fixture::new("codex");
        let _capability = fixture.bind_codex();
        let server = Server::start(&fixture, "idle");
        let recovery = fixture.arm_recovery();
        fs::write(fixture.root().join("composer"), draft).unwrap();
        fixture.refused(&fixture.send(GUARDS), "send-composer-not-empty");
        fixture.unchanged_control_state(&recovery);
        assert!(server.mutations().is_empty());
        assert_eq!(
            fs::read_to_string(fixture.root().join("composer")).unwrap(),
            draft
        );
    }
}

#[test]
fn appserver_turn_started_during_native_admission_refuses_before_input() {
    let fixture = Fixture::new("codex");
    let _capability = fixture.bind_codex();
    let server = Server::start(&fixture, "late-busy");
    let recovery = fixture.arm_recovery();
    fixture.refused(&fixture.send(GUARDS), "send-session-not-idle");
    fixture.unchanged_control_state(&recovery);
    assert!(server.mutations().is_empty());
}

#[test]
fn appserver_draft_after_key_claim_refuses_native_dispatch_and_retains_key() {
    let fixture = Fixture::new("codex");
    let _capability = fixture.bind_codex();
    let server = Server::start(&fixture, "idle");
    let recovery = fixture.arm_recovery();
    fs::write(fixture.root().join("draft-fourth-capture"), "").unwrap();
    let value = fixture.refused(&fixture.send(GUARDS), "send-composer-not-empty");
    assert_eq!(value["error"]["details"]["stage"], "claimed");
    assert_eq!(value["error"]["details"]["pasted"], false);
    assert_eq!(
        fs::read_to_string(fixture.root().join("composer")).unwrap(),
        "user addition"
    );
    fixture.unchanged_control_state(&recovery);
    fixture.refused(&fixture.send(GUARDS), "send-idempotency-key-used");
    assert!(server.mutations().is_empty());
}

#[test]
fn appserver_runtime_identity_change_remains_refused_before_input() {
    let fixture = Fixture::new("codex");
    let _capability = fixture.bind_codex();
    let server = Server::start(&fixture, "idle");
    let recovery = fixture.arm_recovery();
    fs::write(fixture.root().join("flip-second-runtime-read"), "").unwrap();
    fixture.refused(&fixture.send(GUARDS), "send-identity-mismatch");
    fixture.unchanged_control_state(&recovery);
    assert!(server.mutations().is_empty());
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
