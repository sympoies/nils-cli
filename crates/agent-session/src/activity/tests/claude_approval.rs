//! Claude approval waits recorded from hook payloads shaped like Claude Code
//! 2.1.317: `PermissionRequest` carries `prompt_id`, `tool_name` and the same
//! `tool_input` as the preceding `PreToolUse`, but no `tool_use_id`.

use super::*;
use pretty_assertions::assert_eq;

const AT: &str = "2026-11-12T00:00:00Z";
const COMMAND_A: &str = "touch /srv/secret-project/a.txt";
const COMMAND_B: &str = "touch /srv/secret-project/b.txt";

fn hook(name: &str, turn: &str, call: Option<&str>, tool: &str, command: &str) -> TurnEvent {
    let mut raw = json!({
        "hook_event_name": name,
        "session_id": "session-1",
        "prompt_id": turn,
        "tool_name": tool,
        "tool_input": {"command": command, "description": "free text"},
    });
    if let Some(call) = call {
        raw["tool_use_id"] = json!(call);
    }
    normalize_provider_hook(AgentKind::Claude, None, "runtime-1", &raw)
        .unwrap()
        .unwrap()
}

fn start(turn: &str) -> TurnEvent {
    normalize_provider_hook(
        AgentKind::Claude,
        None,
        "runtime-1",
        &json!({"hook_event_name": "UserPromptSubmit", "session_id": "session-1", "prompt_id": turn}),
    )
    .unwrap()
    .unwrap()
}

fn run(document: &mut ActivityDocument, events: impl IntoIterator<Item = TurnEvent>) {
    for event in events {
        reduce(document, &event, AT);
    }
}

fn attention(document: &ActivityDocument) -> Option<&AttentionView> {
    document
        .state
        .current_turn
        .as_ref()
        .and_then(|turn| turn.attention.as_ref())
}

#[test]
fn approval_is_bound_to_its_tool_call_and_cleared_by_the_matching_result() {
    for result in ["PostToolUse", "PostToolUseFailure"] {
        let mut document = document();
        run(
            &mut document,
            [
                start("turn-1"),
                hook("PreToolUse", "turn-1", Some("call-1"), "Bash", COMMAND_A),
                hook("PermissionRequest", "turn-1", None, "Bash", COMMAND_A),
            ],
        );
        assert_eq!(document.state.phase, TurnPhase::NeedsInput, "{result}");
        let waiting = attention(&document).expect("approval wait is recorded");
        assert_eq!(waiting.kind, "approval");
        assert_eq!(waiting.certainty, AttentionCertainty::Exact, "{result}");
        assert_eq!(waiting.extra.get("tool_name"), Some(&json!("Bash")));

        run(
            &mut document,
            [hook(result, "turn-1", Some("call-1"), "Bash", COMMAND_A)],
        );
        assert_eq!(document.state.phase, TurnPhase::Working, "{result}");
        assert!(attention(&document).is_none(), "{result}");
        assert!(document.pending_attention.is_empty(), "{result}");
    }
}

#[test]
fn distinct_parallel_approvals_clear_independently() {
    let mut document = document();
    run(
        &mut document,
        [
            start("turn-1"),
            hook("PreToolUse", "turn-1", Some("call-a"), "Bash", COMMAND_A),
            hook("PreToolUse", "turn-1", Some("call-b"), "Bash", COMMAND_B),
            hook("PermissionRequest", "turn-1", None, "Bash", COMMAND_A),
            hook("PermissionRequest", "turn-1", None, "Bash", COMMAND_B),
        ],
    );
    assert_eq!(attention(&document).unwrap().pending_count, 2);

    run(
        &mut document,
        [hook(
            "PostToolUse",
            "turn-1",
            Some("call-b"),
            "Bash",
            COMMAND_B,
        )],
    );
    assert_eq!(document.state.phase, TurnPhase::NeedsInput);
    assert_eq!(attention(&document).unwrap().pending_count, 1);

    run(
        &mut document,
        [hook(
            "PostToolUse",
            "turn-1",
            Some("call-a"),
            "Bash",
            COMMAND_A,
        )],
    );
    assert_eq!(document.state.phase, TurnPhase::Working);
}

#[test]
fn identical_parallel_calls_cannot_be_told_apart_and_stay_latched() {
    let mut document = document();
    run(
        &mut document,
        [
            start("turn-1"),
            hook("PreToolUse", "turn-1", Some("call-1"), "Bash", COMMAND_A),
            hook("PreToolUse", "turn-1", Some("call-2"), "Bash", COMMAND_A),
            hook("PermissionRequest", "turn-1", None, "Bash", COMMAND_A),
        ],
    );
    let waiting = attention(&document).unwrap();
    assert_eq!(waiting.certainty, AttentionCertainty::Conservative);
    assert_eq!(waiting.extra.get("tool_name"), Some(&json!("Bash")));

    run(
        &mut document,
        [hook(
            "PostToolUse",
            "turn-1",
            Some("call-1"),
            "Bash",
            COMMAND_A,
        )],
    );
    assert_eq!(document.state.phase, TurnPhase::NeedsInput);
}

#[test]
fn approval_without_a_recorded_tool_call_falls_back_to_the_conservative_latch() {
    let mut document = document();
    run(
        &mut document,
        [
            start("turn-1"),
            hook("PermissionRequest", "turn-1", None, "Bash", COMMAND_A),
        ],
    );
    let waiting = attention(&document).unwrap();
    assert_eq!(waiting.certainty, AttentionCertainty::Conservative);
    assert_eq!(waiting.extra.get("tool_name"), Some(&json!("Bash")));

    run(
        &mut document,
        [hook(
            "PostToolUse",
            "turn-1",
            Some("call-1"),
            "Bash",
            COMMAND_A,
        )],
    );
    assert_eq!(document.state.phase, TurnPhase::NeedsInput);
}

#[test]
fn an_approval_binds_only_to_a_call_of_the_same_tool() {
    // The input digest covers the raw tool name, so identical inputs of
    // different tools never match, and a request without a usable name has no
    // digest to match with.
    for (tool, name) in [
        (Some("Write"), "Write"),
        (Some("Bash; rm"), "Bash; rm"),
        (None, ""),
    ] {
        let mut document = document();
        let mut request = json!({
            "hook_event_name": "PermissionRequest", "session_id": "session-1",
            "prompt_id": "turn-1",
            "tool_input": {"command": COMMAND_A, "description": "free text"},
        });
        if let Some(tool) = tool {
            request["tool_name"] = json!(tool);
        }
        let request = normalize_provider_hook(AgentKind::Claude, None, "runtime-1", &request)
            .unwrap()
            .unwrap();
        run(
            &mut document,
            [
                start("turn-1"),
                hook("PreToolUse", "turn-1", Some("call-1"), "Bash", COMMAND_A),
                request,
            ],
        );
        let waiting = attention(&document).unwrap();
        assert_eq!(
            waiting.certainty,
            AttentionCertainty::Conservative,
            "{name}"
        );

        run(
            &mut document,
            [hook(
                "PostToolUse",
                "turn-1",
                Some("call-1"),
                "Bash",
                COMMAND_A,
            )],
        );
        assert_eq!(document.state.phase, TurnPhase::NeedsInput, "{name}");
    }
}

#[test]
fn a_request_does_not_bind_a_call_left_over_from_another_turn() {
    let mut document = document();
    run(
        &mut document,
        [
            start("turn-1"),
            hook("PreToolUse", "turn-1", Some("call-old"), "Bash", COMMAND_A),
            start("turn-2"),
            hook("PermissionRequest", "turn-2", None, "Bash", COMMAND_A),
        ],
    );
    assert_eq!(
        attention(&document).unwrap().certainty,
        AttentionCertainty::Conservative
    );
}

#[test]
fn starting_a_call_in_a_new_turn_drops_the_calls_of_the_old_turn() {
    let mut document = document();
    run(
        &mut document,
        [
            start("turn-1"),
            hook("PreToolUse", "turn-1", Some("call-old"), "Bash", COMMAND_A),
            start("turn-2"),
            hook("PreToolUse", "turn-2", Some("call-new"), "Bash", COMMAND_B),
        ],
    );
    let stored = serde_json::to_value(&document).unwrap();
    assert_eq!(stored["open_tool_calls"].as_array().map(Vec::len), Some(1));
}

#[test]
fn a_second_identical_request_does_not_rebind_a_bound_call() {
    let mut document = document();
    run(
        &mut document,
        [
            start("turn-1"),
            hook("PreToolUse", "turn-1", Some("call-1"), "Bash", COMMAND_A),
            hook("PermissionRequest", "turn-1", None, "Bash", COMMAND_A),
            hook("PermissionRequest", "turn-1", None, "Bash", COMMAND_A),
        ],
    );
    assert_eq!(attention(&document).unwrap().pending_count, 2);

    // The result clears the approval bound to its call; the second request
    // had no call of its own and must remain, conservative.
    run(
        &mut document,
        [hook(
            "PostToolUse",
            "turn-1",
            Some("call-1"),
            "Bash",
            COMMAND_A,
        )],
    );
    let remaining = attention(&document).expect("the unbound request remains");
    assert_eq!(remaining.pending_count, 1);
    assert_eq!(remaining.certainty, AttentionCertainty::Conservative);
}

#[test]
fn an_identical_call_started_later_gets_its_own_binding() {
    let mut document = document();
    run(
        &mut document,
        [
            start("turn-1"),
            hook("PreToolUse", "turn-1", Some("call-1"), "Bash", COMMAND_A),
            hook("PermissionRequest", "turn-1", None, "Bash", COMMAND_A),
            hook("PreToolUse", "turn-1", Some("call-2"), "Bash", COMMAND_A),
            hook("PermissionRequest", "turn-1", None, "Bash", COMMAND_A),
        ],
    );
    run(
        &mut document,
        [hook(
            "PostToolUse",
            "turn-1",
            Some("call-1"),
            "Bash",
            COMMAND_A,
        )],
    );
    let remaining = attention(&document).expect("the second approval remains");
    assert_eq!(remaining.pending_count, 1);
    assert_eq!(remaining.certainty, AttentionCertainty::Exact);

    run(
        &mut document,
        [hook(
            "PostToolUse",
            "turn-1",
            Some("call-2"),
            "Bash",
            COMMAND_A,
        )],
    );
    assert_eq!(document.state.phase, TurnPhase::Working);
}

#[test]
fn evicting_a_bound_call_downgrades_its_approval_to_conservative() {
    // At most 64 open calls are kept. An evicted call can no longer deliver a
    // result, so the approval bound to it must not stay exact.
    let mut document = document();
    run(
        &mut document,
        [
            start("turn-1"),
            hook("PreToolUse", "turn-1", Some("call-0"), "Bash", COMMAND_A),
            hook("PermissionRequest", "turn-1", None, "Bash", COMMAND_A),
        ],
    );
    assert_eq!(
        attention(&document).unwrap().certainty,
        AttentionCertainty::Exact
    );
    for index in 1..=64 {
        run(
            &mut document,
            [hook(
                "PreToolUse",
                "turn-1",
                Some(&format!("call-{index}")),
                "Bash",
                &format!("touch /srv/secret-project/{index}.txt"),
            )],
        );
    }
    let waiting = attention(&document).expect("the approval is still recorded");
    assert_eq!(waiting.certainty, AttentionCertainty::Conservative);
    assert_eq!(waiting.pending_count, 1);
}

#[test]
fn a_lost_pre_tool_use_can_bind_the_request_to_an_identical_open_call() {
    // Known limit, documented in the turn-state contract: when the
    // PreToolUse of the call that asks is lost, an identical open call is the
    // only candidate, so the request binds to it and that call's result
    // clears the approval early. No hook payload shows that a delivery was
    // dropped, so this cannot be detected.
    let mut document = document();
    run(
        &mut document,
        [
            start("turn-1"),
            hook("PreToolUse", "turn-1", Some("call-a1"), "Bash", COMMAND_A),
            // The identical second call's PreToolUse never arrives.
            hook("PermissionRequest", "turn-1", None, "Bash", COMMAND_A),
        ],
    );
    assert_eq!(
        attention(&document).unwrap().certainty,
        AttentionCertainty::Exact
    );
    run(
        &mut document,
        [hook(
            "PostToolUse",
            "turn-1",
            Some("call-a1"),
            "Bash",
            COMMAND_A,
        )],
    );
    assert_eq!(document.state.phase, TurnPhase::Working);
}

#[test]
fn a_result_for_another_turn_does_not_clear_the_approval() {
    let mut document = document();
    run(
        &mut document,
        [
            start("turn-1"),
            hook("PreToolUse", "turn-1", Some("call-1"), "Bash", COMMAND_A),
            hook("PermissionRequest", "turn-1", None, "Bash", COMMAND_A),
            hook("PostToolUse", "turn-2", Some("call-1"), "Bash", COMMAND_A),
        ],
    );
    assert_eq!(document.state.phase, TurnPhase::NeedsInput);
}

#[test]
fn a_denied_approval_stays_recorded_until_the_next_turn_starts() {
    // A manual denial and an interrupt emit no hook (measured on Claude Code
    // 2.1.317), so nothing but the next prompt can end the recorded wait.
    let mut document = document();
    run(
        &mut document,
        [
            start("turn-1"),
            hook("PreToolUse", "turn-1", Some("call-1"), "Bash", COMMAND_A),
            hook("PermissionRequest", "turn-1", None, "Bash", COMMAND_A),
        ],
    );
    assert_eq!(document.state.phase, TurnPhase::NeedsInput);

    run(&mut document, [start("turn-2")]);
    assert_eq!(document.state.phase, TurnPhase::Working);
    assert!(attention(&document).is_none());
    // The abandoned call must not bind or clear anything in the new turn.
    run(
        &mut document,
        [
            hook("PermissionRequest", "turn-2", None, "Bash", COMMAND_A),
            hook("PostToolUse", "turn-2", Some("call-1"), "Bash", COMMAND_A),
        ],
    );
    assert_eq!(document.state.phase, TurnPhase::NeedsInput);
}

#[test]
fn public_state_names_the_tool_but_never_its_input_or_call_id() {
    let mut document = document();
    run(
        &mut document,
        [
            start("turn-1"),
            hook(
                "PreToolUse",
                "turn-1",
                Some("call-secret"),
                "Bash",
                COMMAND_A,
            ),
            hook("PermissionRequest", "turn-1", None, "Bash", COMMAND_A),
        ],
    );
    let public = serde_json::to_string(&document.state).unwrap();
    assert!(public.contains("\"tool_name\":\"Bash\""), "{public}");
    for private in ["secret-project", "free text", "call-secret", "tool_input"] {
        assert!(!public.contains(private), "{private} leaked into {public}");
    }
}

#[test]
fn ingest_keeps_parallel_calls_inside_the_duplicate_window() {
    // The one-second duplicate guard keys on turn and kind, so it must also
    // key on the tool call: a second call started right after the first is a
    // new event, and the approval for it must still bind and clear.
    let tmp = tempfile::TempDir::new().unwrap();
    let clock = super::test_clock::TestClock::freeze();
    let (context, created) = test_session_for_agent(&tmp, AgentKind::Claude);
    activate_runtime(&context, &created.record).unwrap();
    let runtime = &created.record.runtime.as_ref().unwrap().launch_id;
    let event = |name: &str, call: Option<&str>, command: &str| {
        let mut raw = json!({
            "hook_event_name": name, "session_id": "session-1", "prompt_id": "turn-1",
            "tool_name": "Bash", "tool_input": {"command": command},
        });
        if let Some(call) = call {
            raw["tool_use_id"] = json!(call);
        }
        normalize_provider_hook(AgentKind::Claude, None, runtime, &raw)
            .unwrap()
            .unwrap()
    };
    for (name, call, command) in [
        ("UserPromptSubmit", None, COMMAND_A),
        ("PreToolUse", Some("call-1"), COMMAND_A),
        ("PreToolUse", Some("call-2"), COMMAND_B),
        ("PermissionRequest", None, COMMAND_B),
    ] {
        ingest_event(&context, &created.record.id, event(name, call, command)).unwrap();
        clock.advance(Duration::from_millis(50));
    }
    let view = state_for_view(&context, &created.record).unwrap();
    assert_eq!(view.phase, TurnPhase::NeedsInput);
    assert_eq!(
        view.current_turn.unwrap().attention.unwrap().certainty,
        AttentionCertainty::Exact
    );
    ingest_event(
        &context,
        &created.record.id,
        event("PostToolUse", Some("call-2"), COMMAND_B),
    )
    .unwrap();
    assert_eq!(
        state_for_view(&context, &created.record).unwrap().phase,
        TurnPhase::Working
    );
}

#[test]
fn malformed_tool_call_evidence_is_rejected() {
    use agent_hook::tool_call::{ToolCallEvidence, ToolCallPhase};
    let digest = format!("local:v1:{}", "a".repeat(64));
    let evidence =
        |phase, call_id: Option<&str>, input: Option<&str>, name: Option<&str>| ToolCallEvidence {
            phase,
            call_id: call_id.map(str::to_string),
            input_digest: input.map(str::to_string),
            tool_name: name.map(str::to_string),
        };
    let valid = hook("PreToolUse", "turn-1", Some("call-1"), "Bash", COMMAND_A);
    validate_event(&valid, EventAdmission::Generic).expect("normalized evidence is valid");

    let mut wrong_kind = valid.clone();
    wrong_kind.kind = TurnEventKind::TurnStarted;
    let mut wrong_provider = valid.clone();
    wrong_provider.provider = "codex".to_string();
    let mut raw_call_id = valid.clone();
    raw_call_id.tool_call = Some(evidence(
        ToolCallPhase::Started,
        Some("toolu-raw"),
        Some(&digest),
        Some("Bash"),
    ));
    let mut unsafe_name = valid.clone();
    unsafe_name.tool_call = Some(evidence(
        ToolCallPhase::Started,
        Some(&digest),
        Some(&digest),
        Some("Bash; rm -rf"),
    ));
    let mut finished_with_input = valid.clone();
    finished_with_input.tool_call = Some(evidence(
        ToolCallPhase::Finished,
        Some(&digest),
        Some(&digest),
        None,
    ));
    for invalid in [
        wrong_kind,
        wrong_provider,
        raw_call_id,
        unsafe_name,
        finished_with_input,
    ] {
        let error = validate_event(&invalid, EventAdmission::Generic).expect_err("invalid");
        assert_eq!(error.code(), "activity-tool-call-invalid");
    }
}
