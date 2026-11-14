//! Synthetic dispatcher/helper contract; no provider session or credentials.
mod support;
use pretty_assertions::assert_eq;
use serde_json::{Value, json};
use std::{fs, os::unix::fs::PermissionsExt};
use support::Fixture;
const POLICY: &str = r#"schema_version = "agent-hook.policy.v1"
bundle_id = "runtime-kit"
version = "2026.11.14.1"
[[rules]]
id = "session.activity"
products = ["claude"]
events = ["PermissionRequest"]
priority = 10
mode = "enforce"
failure_posture = "closed"
override_class = "locked"
capability = { id = "agent-session.activity.v1", reason_code = "activity-recorded" }
"#;
fn fixture(policy: &str) -> Fixture {
    let f = Fixture::new(policy);
    let helper = f.root.join("helper");
    fs::write(
        &helper,
        r#"#!/bin/sh
if [ "$1" = activity ]; then cat > "$EVENT_FILE"; exit 0; fi
printf '%s\n' "$@" > "$WAIT_ARGS"
printf '%s' "$DECISION_JSON"
"#,
    )
    .unwrap();
    fs::set_permissions(helper, fs::Permissions::from_mode(0o700)).unwrap();
    f
}
fn dispatch(f: &Fixture, decision: &str, extra: &[&str]) -> Value {
    let helper = f.root.join("helper");
    let event = f.root.join("event");
    let args = f.root.join("wait-args");
    let mut cli = vec!["dispatch", "--product", "claude", "--format", "provider"];
    cli.extend_from_slice(extra);
    let out=f.run_with_env(&cli, Some(r#"{"hook_event_name":"PermissionRequest","session_id":"provider-fixture","prompt_id":"turn-fixture","tool_name":"Bash","tool_input":{"command":"fixture-input-canary"}}"#), &[
        ("AGENT_SESSION_BIN", helper.to_str().unwrap()), ("AGENT_SESSION_ID", "managed-session"),
        ("AGENT_SESSION_RUNTIME_ID", "managed-runtime"), ("EVENT_FILE",event.to_str().unwrap()),
        ("WAIT_ARGS",args.to_str().unwrap()), ("DECISION_JSON",decision)]);
    assert_eq!(out.code, 0, "{}", out.stderr_text());
    out.stdout_json()
}
#[test]
fn permission_dispatch_relays_only_single_call_allow_and_deny() {
    for behavior in ["allow", "deny"] {
        let f = fixture(POLICY);
        let expected = json!({"hookSpecificOutput":{"hookEventName":"PermissionRequest","decision":{"behavior":behavior}}});
        assert_eq!(dispatch(&f, &expected.to_string(), &[]), expected);
        let event: Value =
            serde_json::from_slice(&fs::read(f.root.join("event")).unwrap()).unwrap();
        let args = fs::read_to_string(f.root.join("wait-args")).unwrap();
        assert!(args.contains(&format!(
            "--attention-id\n{}\n",
            event["attention_id"].as_str().unwrap()
        )));
        assert!(args.contains("--runtime-id\nmanaged-runtime\n"));
        assert!(!args.contains("fixture-input-canary"));
        assert!(!event.to_string().contains("fixture-input-canary"));
    }
}
#[test]
fn permission_dispatch_cannot_widen_permission_or_override_policy_block() {
    let f = fixture(POLICY);
    let invalid = json!({"hookSpecificOutput":{"hookEventName":"PermissionRequest","decision":{"behavior":"allow","updatedPermissions":[]}}});
    assert!(dispatch(&f, &invalid.to_string(), &[])["hookSpecificOutput"]["decision"].is_null());
    let blocked = POLICY.replace("agent-session.activity.v1", "decision.block.v1");
    let f = fixture(&blocked);
    let result = dispatch(&f, "{}", &[]);
    assert_eq!(result["hookSpecificOutput"]["decision"]["behavior"], "deny");
    assert!(!f.root.join("wait-args").exists());
    let transformed = POLICY
        .replace("agent-session.activity.v1", "decision.transform.v1")
        .replace(
            "reason_code = \"activity-recorded\"",
            "reason_code = \"rewrite\", replacement = {command = \"safe-fixture-command\"}",
        );
    let f = fixture(&transformed);
    let result = dispatch(&f, "{}", &[]);
    assert_eq!(
        result["hookSpecificOutput"]["decision"]["updatedInput"],
        json!({"command":"safe-fixture-command"})
    );
    assert!(!f.root.join("wait-args").exists());
}
#[test]
fn permission_shadow_dispatch_never_waits_or_answers() {
    let f = fixture(POLICY);
    let expected = json!({"hookSpecificOutput":{"hookEventName":"PermissionRequest","decision":{"behavior":"allow"}}});
    let result = dispatch(&f, &expected.to_string(), &["--shadow"]);
    assert!(result["hookSpecificOutput"]["decision"].is_null());
    assert!(!f.root.join("wait-args").exists());
}
