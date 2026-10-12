use super::*;
use pretty_assertions::assert_eq;

#[test]
fn claude_permission_latch_preserves_parallel_requests_and_wrong_turn() {
    for parallel_tool in ["Bash", "Read"] {
        let mut document = document();
        let hook = |name: &str, tool: &str, turn: &str| {
            normalize_provider_hook(AgentKind::Claude, None, "runtime-1", &json!({
                "hook_event_name": name, "session_id": "session-1", "prompt_id": turn, "tool_name": tool,
            })).unwrap().unwrap()
        };
        for event in [
            hook("UserPromptSubmit", "Bash", "turn-1"),
            hook("PermissionRequest", "Bash", "turn-1"),
            hook("PermissionRequest", parallel_tool, "turn-1"),
            hook("PostToolUse", "Bash", "turn-1"),
        ] {
            reduce(&mut document, &event, "2026-10-07T00:00:00Z");
        }
        assert_eq!(document.state.phase, TurnPhase::NeedsInput);
        assert_eq!(document.pending_attention.len(), 2);
        let public = serde_json::to_string(&document.state).unwrap();
        assert!(!public.contains("permission_tool_id"));
    }
    let mut document = document();
    for raw in [
        json!({"hook_event_name": "UserPromptSubmit", "prompt_id": "turn-1"}),
        json!({"hook_event_name": "PermissionRequest", "prompt_id": "turn-1", "tool_name": "Bash"}),
        json!({"hook_event_name": "PostToolUse", "prompt_id": "other-turn", "tool_name": "Bash"}),
    ] {
        let event = normalize_provider_hook(AgentKind::Claude, None, "runtime-1", &raw)
            .unwrap()
            .unwrap();
        reduce(&mut document, &event, "2026-10-07T00:00:00Z");
    }
    assert_eq!(document.state.phase, TurnPhase::NeedsInput);
}

#[test]
fn claude_permission_latch_waits_for_same_turn_stop() {
    for (event_name, tool, expected) in [
        ("PostToolUse", "Bash", TurnPhase::NeedsInput),
        ("PostToolUseFailure", "Bash", TurnPhase::NeedsInput),
        ("PreToolUse", "Bash", TurnPhase::NeedsInput),
        ("PermissionDenied", "Bash", TurnPhase::NeedsInput),
        ("PostToolUse", "Read", TurnPhase::NeedsInput),
        ("Stop", "Bash", TurnPhase::Working),
    ] {
        let tmp = tempfile::TempDir::new().unwrap();
        let (context, created) = test_session_for_agent(&tmp, AgentKind::Claude);
        activate_runtime(&context, &created.record).unwrap();
        let runtime = &created.record.runtime.as_ref().unwrap().launch_id;
        for (name, tool) in [
            ("UserPromptSubmit", "Bash"),
            ("PermissionRequest", "Bash"),
            (event_name, tool),
        ] {
            let raw = json!({"hook_event_name": name, "session_id": "session-1", "prompt_id": "turn-1", "tool_name": tool});
            if let Some(event) =
                normalize_provider_hook(AgentKind::Claude, None, runtime, &raw).unwrap()
            {
                ingest_event(&context, &created.record.id, event).unwrap();
            }
        }
        assert_eq!(
            state_for_view(&context, &created.record).unwrap().phase,
            expected,
            "{event_name}/{tool}"
        );
    }
}

#[test]
fn claude_permission_stop_preserves_uncorrelated_and_nonapproval_attention() {
    for mode in ["wrong-turn", "missing-turn", "clarification", "overflow"] {
        let mut document = document();
        for raw in [
            json!({"hook_event_name":"UserPromptSubmit", "prompt_id":"turn-1"}),
            json!({"hook_event_name":"PermissionRequest", "prompt_id":"turn-1", "tool_name":"Bash"}),
        ] {
            let event = normalize_provider_hook(AgentKind::Claude, None, "runtime-1", &raw)
                .unwrap()
                .unwrap();
            reduce(&mut document, &event, "2026-10-07T00:00:00Z");
        }
        let mut raw = json!({"hook_event_name":"Stop", "prompt_id":"turn-1"});
        match mode {
            "wrong-turn" => raw["prompt_id"] = json!("other-turn"),
            "missing-turn" => {
                raw.as_object_mut().unwrap().remove("prompt_id");
            }
            "clarification" => document.pending_attention[0].kind = "clarification".into(),
            "overflow" => {
                document.overflow_attention = Some(
                    serde_json::from_value(json!({
                        "kind":"approval", "requested_at":"2026-10-07T00:00:00Z", "count":1
                    }))
                    .unwrap(),
                )
            }
            _ => unreachable!(),
        }
        let event = normalize_provider_hook(AgentKind::Claude, None, "runtime-1", &raw)
            .unwrap()
            .unwrap();
        reduce(&mut document, &event, "2026-10-07T00:00:01Z");
        assert_eq!(document.state.phase, TurnPhase::NeedsInput, "{mode}");
        assert!(
            document
                .state
                .current_turn
                .as_ref()
                .unwrap()
                .attention
                .is_some(),
            "{mode}"
        );
    }
    let malformed = normalize_provider_hook(
        AgentKind::Claude,
        None,
        "runtime-1",
        &json!({
            "hook_event_name":"PostToolUse", "prompt_id":"turn-1", "tool_name":{"unexpected":true}
        }),
    );
    assert!(
        malformed.is_ok(),
        "optional tool metadata invalidated a progress event"
    );
}
