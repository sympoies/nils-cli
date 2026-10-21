//! Codex `Interrupt` hook event, per the pinned upstream schema
//! `codex-rs/hooks/schema/generated/interrupt.command.input.schema.json`
//! (openai/codex tag `rust-v0.162.1`; identical at `rust-v0.150.0`, which
//! introduced the event). The input carries `cwd`, `hook_event_name`, `model`,
//! `permission_mode`, `session_id`, `transcript_path`, and `turn_id`; the output
//! schema only admits `systemMessage`.

#[cfg(test)]
mod support;

use std::fs;
use std::os::unix::fs::PermissionsExt;

use pretty_assertions::assert_eq;

use support::Fixture;

fn policy(products: &str, event: &str, capability: &str) -> String {
    format!(
        r#"schema_version = "agent-hook.policy.v1"
bundle_id = "runtime-kit"
version = "2026.07.20.1"

[[rules]]
id = "session.activity"
products = [{products}]
events = ["{event}"]
priority = 10
mode = "enforce"
failure_posture = "closed"
override_class = "locked"
capability = {capability}
"#
    )
}

const ACTIVITY: &str = r#"{ id = "agent-session.activity.v1", reason_code = "activity-recorded" }"#;

fn interrupt_payload(extra: &str) -> String {
    format!(
        r#"{{"hook_event_name":"Interrupt","cwd":"/work","model":"model-x","permission_mode":"default","session_id":"provider-session-secret","transcript_path":null,"turn_id":"provider-turn-secret"{extra}}}"#
    )
}

#[test]
fn policy_registering_codex_interrupt_activity_validates() {
    let fixture = Fixture::new(&policy(r#""codex""#, "Interrupt", ACTIVITY));
    let output = fixture.run(&["validate", "--format", "json"], None);
    assert_eq!(output.code, 0, "envelope={}", output.stdout_text());
}

#[test]
fn interrupt_is_codex_only_and_cannot_carry_context_or_block() {
    for (products, capability, code) in [
        // Claude has no Interrupt event.
        (r#""claude""#, ACTIVITY, "policy-event-unsupported"),
        // The upstream output schema only admits `systemMessage`, so a rule
        // that could emit a block or context action is rejected up front.
        (
            r#""codex""#,
            r#"{ id = "decision.block.v1", reason_code = "x-blocked", message = "blocked" }"#,
            "policy-capability-event-unsupported",
        ),
        (
            r#""codex""#,
            r#"{ id = "decision.context.v1", reason_code = "x-context", text = "context" }"#,
            "policy-capability-event-unsupported",
        ),
    ] {
        let fixture = Fixture::new(&policy(products, "Interrupt", capability));
        let output = fixture.run(&["validate", "--format", "json"], None);
        assert_eq!(
            output.code, 65,
            "products={products} capability={capability}"
        );
        assert_eq!(output.stdout_json()["error"]["code"], code);
    }
}

struct Captured {
    code: i32,
    stderr: String,
    stdout: String,
    event: Option<String>,
}

fn dispatch_interrupt(payload: &str, format: &str) -> Captured {
    let fixture = Fixture::new(&policy(r#""codex""#, "Interrupt", ACTIVITY));
    let fake = fixture.root.join("agent-session-fake");
    let input = fixture.root.join("activity.json");
    fs::write(
        &fake,
        "#!/bin/sh\nif [ \"$2\" = hook ]; then exit 0; fi\ndd of=\"$CAPTURE_STDIN\" status=none\n",
    )
    .expect("fake agent-session");
    fs::set_permissions(&fake, fs::Permissions::from_mode(0o700)).expect("fake mode");
    let result = fixture.run_with_env(
        &["dispatch", "--product", "codex", "--format", format],
        Some(payload),
        &[
            ("AGENT_SESSION_BIN", fake.to_str().expect("fake path")),
            ("AGENT_SESSION_ID", "managed-session"),
            ("AGENT_SESSION_RUNTIME_ID", "managed-runtime"),
            ("CAPTURE_STDIN", input.to_str().expect("stdin path")),
        ],
    );
    Captured {
        code: result.code,
        stderr: result.stderr_text(),
        stdout: result.stdout_text(),
        event: fs::read_to_string(&input).ok(),
    }
}

#[test]
fn interrupt_normalizes_to_a_metadata_only_stop_observation() {
    let captured = dispatch_interrupt(&interrupt_payload(""), "json");
    assert_eq!(captured.code, 0, "stderr={}", captured.stderr);
    let serialized = captured.event.expect("activity event was forwarded");
    let event: serde_json::Value = serde_json::from_str(&serialized).expect("event JSON");
    assert_eq!(event["schema_version"], "agent-session.turn-event.v1");
    assert_eq!(event["runtime_id"], "managed-runtime");
    assert_eq!(event["provider"], "codex");
    assert_eq!(event["kind"], "stop_observed");
    assert_eq!(event["confidence"], "observed");
    assert_eq!(event["source_kind"], "provider_hook");
    assert_eq!(event.get("completion_candidate"), None);
    for field in ["provider_session_id", "provider_turn_id"] {
        assert!(
            event[field]
                .as_str()
                .is_some_and(|value| value.starts_with("local:v1:")),
            "{field} was not projected: {serialized}"
        );
    }
    for leaked in [
        "provider-session-secret",
        "provider-turn-secret",
        "/work",
        "model-x",
    ] {
        assert!(!serialized.contains(leaked), "leaked {leaked}");
    }
}

#[test]
fn accepted_interrupt_renders_an_empty_provider_output() {
    let captured = dispatch_interrupt(&interrupt_payload(""), "provider");
    assert_eq!(captured.code, 0, "stderr={}", captured.stderr);
    assert_eq!(captured.stdout.trim(), "{}");
}

#[test]
fn malformed_interrupt_payloads_fail_closed_without_activity() {
    // The typed per-field errors are asserted by the adapter unit tests; here
    // the dispatch result must be a closed decision and forward nothing.
    for payload in [
        r#"{"hook_event_name":"Interrupt","session_id":"s1"}"#,
        r#"{"hook_event_name":"Interrupt","turn_id":"t1"}"#,
        r#"{"hook_event_name":"Interrupt","session_id":"s1","turn_id":""}"#,
        r#"{"hook_event_name":"Interrupt","session_id":"s1","turn_id":7}"#,
    ] {
        let captured = dispatch_interrupt(payload, "json");
        assert!(captured.event.is_none(), "forwarded activity for {payload}");
        let envelope: serde_json::Value =
            serde_json::from_str(captured.stdout.trim()).expect("dispatch envelope");
        assert_eq!(envelope["data"]["action"], "block", "payload={payload}");
        assert_eq!(
            envelope["data"]["reasons"][0]["code"],
            "session.activity:capability-failure-closed"
        );
    }
    let invalid_root = dispatch_interrupt(r#"["Interrupt"]"#, "json");
    assert!(invalid_root.event.is_none());
    assert!(invalid_root.stdout.contains("provider-input-invalid"));
}

#[test]
fn interrupt_denial_uses_the_upstream_output_shape() {
    let captured = dispatch_interrupt(
        r#"{"hook_event_name":"Interrupt","session_id":"s1"}"#,
        "provider",
    );
    assert_eq!(captured.code, 0, "stderr={}", captured.stderr);
    assert_eq!(
        serde_json::from_str::<serde_json::Value>(captured.stdout.trim()).unwrap(),
        serde_json::json!({"systemMessage": "agent-hook:session.activity:capability-failure-closed"})
    );
}

#[test]
fn claude_payload_naming_interrupt_is_unsupported() {
    let fixture = Fixture::new(&policy(r#""codex""#, "Interrupt", ACTIVITY));
    let output = fixture.run(
        &["dispatch", "--product", "claude", "--format", "json"],
        Some(r#"{"hook_event_name":"Interrupt","session_id":"s1","turn_id":"t1"}"#),
    );
    assert!(output.code != 0);
    assert!(output.stdout_text().contains("provider-event-unsupported"));
}
