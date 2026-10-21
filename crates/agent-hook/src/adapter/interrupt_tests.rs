//! Codex `Interrupt` activity normalization against the pinned upstream input
//! schema (openai/codex `rust-v0.162.1`, `interrupt.command.input`).

use super::*;

fn activity(payload: &Value) -> Result<Option<Value>, HookError> {
    let raw = serde_json::to_vec(payload).unwrap();
    let request = normalize(Product::Codex, None, &raw)?;
    Ok(normalize_activity_event(&request, &raw, "runtime-1")?
        .map(|bytes| serde_json::from_slice(&bytes).unwrap()))
}

fn payload() -> Value {
    json!({
        "hook_event_name": "Interrupt", "cwd": "/work", "model": "model-x",
        "permission_mode": "default", "session_id": "session-1",
        "transcript_path": null, "turn_id": "turn-1",
    })
}

#[test]
fn interrupt_normalizes_to_an_observed_stop_with_projected_ids() {
    let event = activity(&payload()).unwrap().expect("activity event");
    assert_eq!(event["kind"], "stop_observed");
    assert_eq!(event["provider"], "codex");
    assert_eq!(event["confidence"], "observed");
    assert!(event.get("completion_candidate").is_none());
    assert!(
        event["provider_turn_id"]
            .as_str()
            .is_some_and(|id| id != "turn-1")
    );
}

#[test]
fn interrupt_without_a_required_identifier_fails_closed() {
    for field in ["session_id", "turn_id"] {
        let mut value = payload();
        value.as_object_mut().unwrap().remove(field);
        assert_eq!(
            activity(&value).unwrap_err().code,
            "provider-hook-correlation-missing",
            "missing {field}"
        );
        value[field] = json!("");
        assert_eq!(
            activity(&value).unwrap_err().code,
            "provider-hook-correlation-missing",
            "empty {field}"
        );
        value[field] = json!(7);
        assert_eq!(
            activity(&value).unwrap_err().code,
            "provider-hook-identifier-invalid",
            "non-string {field}"
        );
    }
}

#[test]
fn interrupt_denial_renders_only_the_upstream_system_message() {
    assert_eq!(
        provider_denial(Product::Codex, "Interrupt", "activity-failed"),
        json!({"systemMessage": "agent-hook:activity-failed"})
    );
}
