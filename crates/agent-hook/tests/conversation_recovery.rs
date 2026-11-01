//! Event-scoped degradation and recovery across real hook processes.
mod support;

use std::{fs, os::unix::fs::PermissionsExt};

use pretty_assertions::{assert_eq, assert_ne};
use serde_json::{Value, json};
use support::Fixture;

const POLICY: &str = r#"schema_version = "agent-hook.policy.v1"
bundle_id = "runtime-kit"
version = "2026.07.20.1"
[[rules]]
id = "fixture.activity"
products = ["codex", "claude"]
events = ["UserPromptSubmit", "PreToolUse"]
priority = 100
mode = "enforce"
failure_posture = "closed"
override_class = "locked"
capability = { id = "agent-session.activity.v1", reason_code = "activity-recorded" }
[[rules]]
id = "fixture.other"
products = ["codex", "claude"]
events = ["SessionStart"]
priority = 110
mode = "enforce"
failure_posture = "closed"
override_class = "locked"
capability = { id = "decision.allow.v1", reason_code = "other-healthy" }
"#;

fn helper(fixture: &Fixture, succeeds: bool) {
    let path = fixture.root.join("activity-helper");
    fs::write(
        &path,
        if succeeds {
            "#!/bin/sh\nexit 0\n"
        } else {
            "#!/bin/sh\nexit 65\n"
        },
    )
    .unwrap();
    fs::set_permissions(path, fs::Permissions::from_mode(0o700)).unwrap();
}

fn dispatch(fixture: &Fixture, product: &str, session: &str, runtime: &str, event: Value) -> Value {
    let helper_path = fixture.root.join("activity-helper");
    let result = fixture.run_with_env(
        &["dispatch", "--product", product, "--format", "json"],
        Some(&event.to_string()),
        &[
            ("AGENT_SESSION_ID", session),
            ("AGENT_SESSION_RUNTIME_ID", runtime),
            ("AGENT_SESSION_BIN", helper_path.to_str().unwrap()),
            ("AGENT_SESSION_COORDINATION_MODE", "enforce"),
        ],
    );
    assert_eq!(
        result.code,
        0,
        "{product} {event}: {} {}",
        result.stdout_text(),
        result.stderr_text()
    );
    result.stdout_json()["data"].clone()
}

fn prompt() -> Value {
    json!({"hook_event_name":"UserPromptSubmit", "prompt":"fixture request"})
}

fn recovered(decision: &Value) -> usize {
    decision["reasons"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|reason| reason["code"] == "conversation-degradation-recovered")
        .count()
}

#[test]
fn activity_degraded_then_healthy_then_healthy_recovers_once_in_the_same_session() {
    for product in ["codex", "claude"] {
        let fixture = Fixture::new(POLICY);
        helper(&fixture, true);
        let never_degraded = dispatch(&fixture, product, "session-a", "runtime-a", prompt());
        assert_eq!(never_degraded["action"], "allow");
        assert_eq!(recovered(&never_degraded), 0);

        helper(&fixture, false);
        let degraded = dispatch(&fixture, product, "session-a", "runtime-a", prompt());
        assert_eq!(degraded["action"], "warn");
        assert!(
            degraded["reasons"]
                .as_array()
                .unwrap()
                .iter()
                .any(|reason| reason["code"] == "coordination-degraded-read-only")
        );

        helper(&fixture, true);
        let unrelated = dispatch(
            &fixture,
            product,
            "session-a",
            "runtime-a",
            json!({"hook_event_name":"SessionStart"}),
        );
        let unselected = dispatch(
            &fixture,
            product,
            "session-a",
            "runtime-a",
            json!({"hook_event_name":"PostToolUse", "tool_name":"Bash", "tool_input":{"command":"true"}}),
        );
        let other_session = dispatch(&fixture, product, "session-b", "runtime-a", prompt());
        let other_runtime = dispatch(&fixture, product, "session-a", "runtime-b", prompt());
        let other_product = dispatch(
            &fixture,
            if product == "codex" {
                "claude"
            } else {
                "codex"
            },
            "session-a",
            "runtime-a",
            prompt(),
        );
        let first_healthy = dispatch(&fixture, product, "session-a", "runtime-a", prompt());
        let next_healthy = dispatch(&fixture, product, "session-a", "runtime-a", prompt());
        for decision in [
            &other_session,
            &other_runtime,
            &other_product,
            &next_healthy,
            &unrelated,
            &unselected,
        ] {
            assert_eq!(decision["action"], "allow");
        }
        assert_eq!(recovered(&other_session), 0);
        assert_eq!(recovered(&other_runtime), 0);
        assert_eq!(recovered(&other_product), 0);
        assert_eq!(recovered(&unrelated), 0);
        assert_eq!(recovered(&unselected), 0);
        assert!(
            first_healthy["reasons"]
                .as_array()
                .unwrap()
                .iter()
                .any(|reason| reason["code"] == "activity-recorded"
                    && reason["disposition"] == "allow")
        );
        assert_eq!(
            recovered(&first_healthy),
            1,
            "the next successful selected evaluation must recover: {first_healthy}"
        );
        assert_eq!(recovered(&next_healthy), 0);
        let context = first_healthy["context"].as_str().unwrap();
        assert!(context.contains("this event"));
        assert!(context.contains("individually"));
        let context = degraded["context"].as_str().unwrap();
        assert!(context.contains("this event only"));
        assert!(context.contains("individually"));
    }
}

fn coordination(fixture: &Fixture) {
    use nils_common::coordination_projection::{CLAIM_VERSION, COALESCING_REGISTRY_VERSION};
    let now = jiff::Timestamp::now().as_second();
    let directory = fixture.session_state.join("coordination");
    fs::create_dir_all(&directory).unwrap();
    fs::set_permissions(&directory, fs::Permissions::from_mode(0o700)).unwrap();
    let registry = directory.join("registry.json");
    fs::write(
        &registry,
        serde_json::to_vec(&json!({
            "schema_version":COALESCING_REGISTRY_VERSION, "fingerprint_epoch":1,
            "fingerprint_key":"0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef",
            "brokers":{"session-a":{"session_id":"session-a", "incarnation":"runtime-a",
                "state":"ready", "coordination_mode":"enforce", "heartbeat_epoch":now}},
            "claims":[{"schema_version":CLAIM_VERSION, "session_id":"session-a",
                "session_incarnation":"runtime-a", "state":"active", "expires_at_epoch":now+600}]
        }))
        .unwrap(),
    )
    .unwrap();
    Fixture::set_private(&registry);
    let session = fixture.session_state.join("sessions/session-a");
    fs::create_dir_all(session.join("coordination")).unwrap();
    let record = session.join("session.json");
    fs::write(
        &record,
        json!({"schema_version":"agent-session.session.v1", "id":"session-a",
        "coordination_mode":"enforce", "runtime":{"launch_id":"runtime-a"}})
        .to_string(),
    )
    .unwrap();
    Fixture::set_private(&record);
    let heartbeat = session.join("coordination/heartbeat");
    fs::write(&heartbeat, format!("runtime-a:{now}\n")).unwrap();
    Fixture::set_private(&heartbeat);
}

#[test]
fn coordination_degradation_recovers_only_after_a_successful_selected_projection() {
    let policy = POLICY.replace(
        "agent-session.activity.v1",
        "agent-session.semantic-conflict.v1",
    );
    for product in ["codex", "claude"] {
        let fixture = Fixture::new(&policy);
        coordination(&fixture);
        let clean = dispatch(&fixture, product, "session-a", "runtime-a", prompt());
        assert_eq!(clean["action"], "allow");
        assert_eq!(recovered(&clean), 0);
        let registry = fixture.session_state.join("coordination/registry.json");
        fs::write(&registry, b"{").unwrap();
        let degraded = dispatch(&fixture, product, "session-a", "runtime-a", prompt());
        assert_eq!(degraded["action"], "warn");
        fs::remove_file(&registry).unwrap();
        let absent = dispatch(&fixture, product, "session-a", "runtime-a", prompt());
        assert_eq!(
            recovered(&absent),
            0,
            "a missing registry is not successful coordination"
        );
        coordination(&fixture);
        let healthy = dispatch(&fixture, product, "session-a", "runtime-a", prompt());
        let next = dispatch(&fixture, product, "session-a", "runtime-a", prompt());
        assert_eq!(recovered(&healthy), 1);
        assert_eq!(recovered(&next), 0);
        let context = degraded["context"].as_str().unwrap();
        assert!(context.contains("this event only"));
        assert!(context.contains("individually"));
    }
}

#[test]
fn recovery_context_reaches_the_provider_once_without_a_block() {
    for product in ["codex", "claude"] {
        let fixture = Fixture::new(POLICY);
        helper(&fixture, false);
        dispatch(&fixture, product, "session-a", "runtime-a", prompt());
        helper(&fixture, true);
        let helper_path = fixture.root.join("activity-helper");
        let native = || {
            let out = fixture.run_with_env(
                &["dispatch", "--product", product, "--format", "provider"],
                Some(&prompt().to_string()),
                &[
                    ("AGENT_SESSION_ID", "session-a"),
                    ("AGENT_SESSION_RUNTIME_ID", "runtime-a"),
                    ("AGENT_SESSION_BIN", helper_path.to_str().unwrap()),
                ],
            );
            assert_eq!(out.code, 0, "{}", out.stderr_text());
            out.stdout_json()
        };
        let first = native();
        let second = native();
        assert!(
            first["hookSpecificOutput"]["additionalContext"]
                .as_str()
                .is_some_and(|text| text.contains("recovery") && text.contains("individually")),
            "{first}"
        );
        assert!(first.get("decision").is_none(), "{first}");
        assert_eq!(second, json!({}));
    }
}

#[test]
fn conversation_recovery_does_not_admit_a_failed_mutation() {
    let fixture = Fixture::new(POLICY);
    helper(&fixture, false);
    dispatch(&fixture, "codex", "session-a", "runtime-a", prompt());
    let helper_path = fixture.root.join("activity-helper");
    let result = fixture.run_with_env(
        &["dispatch", "--product", "codex", "--format", "json"],
        Some(
            &json!({"hook_event_name":"PreToolUse", "tool_name":"Write", "cwd":fixture.root,
            "tool_input":{"path":fixture.root.join("output.txt"), "content":"fixture"}})
            .to_string(),
        ),
        &[
            ("AGENT_SESSION_ID", "session-a"),
            ("AGENT_SESSION_RUNTIME_ID", "runtime-a"),
            ("AGENT_SESSION_BIN", helper_path.to_str().unwrap()),
            ("AGENT_SESSION_COORDINATION_MODE", "enforce"),
        ],
    );
    assert_ne!(result.code, 0);
    assert!(
        !result
            .stdout_text()
            .contains("conversation-degradation-recovered")
    );
    helper(&fixture, true);
    let healthy = dispatch(&fixture, "codex", "session-a", "runtime-a", prompt());
    assert_eq!(
        recovered(&healthy),
        1,
        "a failed mutation must not consume pending recovery"
    );
}
