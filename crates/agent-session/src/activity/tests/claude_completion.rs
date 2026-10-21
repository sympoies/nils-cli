use super::test_clock::TestClock;
use super::*;
use pretty_assertions::assert_eq;

#[test]
fn completion_candidate_requires_observed_claude_stop_hook() {
    let mut valid = event(TurnEventKind::StopObserved, "completion-candidate");
    valid.provider = "claude".to_string();
    valid.completion_candidate = true;
    validate_event(&valid, EventAdmission::Generic).expect("qualifying Claude Stop candidate");

    let mut invalid_provider = valid.clone();
    invalid_provider.provider = "codex".to_string();
    let mut invalid_kind = valid.clone();
    invalid_kind.kind = TurnEventKind::Progress;
    let mut invalid_source = valid.clone();
    invalid_source.source_kind = SourceKind::Runtime;
    let mut invalid_confidence = valid;
    invalid_confidence.confidence = Confidence::Inferred;

    for invalid in [
        invalid_provider,
        invalid_kind,
        invalid_source,
        invalid_confidence,
    ] {
        let error = validate_event(&invalid, EventAdmission::Generic)
            .expect_err("non-qualifying completion candidate");
        assert_eq!(error.code(), "activity-completion-candidate-invalid");
    }
}

#[test]
fn claude_stop_completion_without_idle_prompt() {
    for (event_name, stop_fields, expected) in [
        (
            "Stop",
            json!({"stop_hook_active": false, "background_tasks": []}),
            TurnPhase::Waiting,
        ),
        (
            "Stop",
            json!({"stop_hook_active": true, "background_tasks": []}),
            TurnPhase::Working,
        ),
        (
            "Stop",
            json!({"stop_hook_active": false, "background_tasks": [{"id": "task-1"}]}),
            TurnPhase::Working,
        ),
        ("Stop", json!({}), TurnPhase::Working),
        (
            "StopFailure",
            json!({"error": "server_error"}),
            TurnPhase::Waiting,
        ),
    ] {
        let tmp = tempfile::TempDir::new().expect("tempdir");
        let clock = TestClock::freeze();
        let (context, created) = test_session_for_agent(&tmp, AgentKind::Claude);
        activate_runtime(&context, &created.record).expect("activate");
        let runtime = &created.record.runtime.as_ref().expect("runtime").launch_id;
        let hook = |name: &str, fields: Value| {
            let mut raw =
                json!({"hook_event_name": name, "session_id": "session-1", "turn_id": "turn-1"});
            raw.as_object_mut()
                .unwrap()
                .extend(fields.as_object().unwrap().clone());
            normalize_provider_hook(AgentKind::Claude, None, runtime, &raw)
                .unwrap()
                .unwrap()
        };
        ingest_event(
            &context,
            &created.record.id,
            hook("UserPromptSubmit", json!({})),
        )
        .unwrap();
        clock.advance(Duration::from_millis(10));
        let initial =
            ingest_event(&context, &created.record.id, hook(event_name, stop_fields)).unwrap();
        if event_name == "Stop" {
            assert_eq!(initial.turn_state.phase, TurnPhase::Working);
        }
        clock.advance(Duration::from_millis(1100));
        let state = state_for_view(&context, &created.record).unwrap();
        assert_eq!(state.phase, expected, "{event_name}");
        if expected == TurnPhase::Waiting {
            assert!(state.current_turn.is_none());
        }
    }
}

#[test]
fn claude_stop_continuation_cancels_completion_candidate() {
    let tmp = tempfile::TempDir::new().expect("tempdir");
    let clock = TestClock::freeze();
    let (context, created) = test_session_for_agent(&tmp, AgentKind::Claude);
    activate_runtime(&context, &created.record).unwrap();
    let runtime = &created.record.runtime.as_ref().unwrap().launch_id;
    for raw in [
        json!({"hook_event_name": "UserPromptSubmit"}),
        json!({"hook_event_name": "Stop", "stop_hook_active": false, "background_tasks": []}),
        json!({"hook_event_name": "PreToolUse", "tool_name": "Bash"}),
    ] {
        let mut raw = raw;
        raw["session_id"] = json!("session-1");
        raw["turn_id"] = json!("turn-1");
        let event = normalize_provider_hook(AgentKind::Claude, None, runtime, &raw)
            .unwrap()
            .unwrap();
        ingest_event(&context, &created.record.id, event).unwrap();
        clock.advance(Duration::from_millis(100));
    }
    clock.advance(Duration::from_millis(1100));
    assert_eq!(
        state_for_view(&context, &created.record).unwrap().phase,
        TurnPhase::Working
    );
}

#[test]
fn claude_reentrant_stop_cancels_completion_candidate() {
    let tmp = tempfile::TempDir::new().expect("tempdir");
    let clock = TestClock::freeze();
    let (context, created) = test_session_for_agent(&tmp, AgentKind::Claude);
    activate_runtime(&context, &created.record).unwrap();
    let runtime = &created.record.runtime.as_ref().unwrap().launch_id;
    for raw in [
        json!({"hook_event_name": "UserPromptSubmit"}),
        json!({"hook_event_name": "Stop", "stop_hook_active": false, "background_tasks": []}),
        json!({"hook_event_name": "Stop", "stop_hook_active": true, "background_tasks": []}),
    ] {
        let mut raw = raw;
        raw["session_id"] = json!("session-1");
        raw["turn_id"] = json!("turn-1");
        let event = normalize_provider_hook(AgentKind::Claude, None, runtime, &raw)
            .unwrap()
            .unwrap();
        ingest_event(&context, &created.record.id, event).unwrap();
        clock.advance(Duration::from_millis(100));
    }
    clock.advance(Duration::from_millis(1100));
    assert_eq!(
        state_for_view(&context, &created.record).unwrap().phase,
        TurnPhase::Working
    );
}

#[test]
fn claude_stop_completes_on_the_real_clock_after_the_quiet_window() {
    let tmp = tempfile::TempDir::new().expect("tempdir");
    let (context, created) = test_session_for_agent(&tmp, AgentKind::Claude);
    activate_runtime(&context, &created.record).unwrap();
    let runtime = &created.record.runtime.as_ref().unwrap().launch_id;
    for raw in [
        json!({"hook_event_name": "UserPromptSubmit"}),
        json!({"hook_event_name": "Stop", "stop_hook_active": false, "background_tasks": []}),
    ] {
        let mut raw = raw;
        raw["session_id"] = json!("session-1");
        raw["turn_id"] = json!("turn-1");
        let event = normalize_provider_hook(AgentKind::Claude, None, runtime, &raw)
            .unwrap()
            .unwrap();
        ingest_event(&context, &created.record.id, event).unwrap();
    }
    // The quiet window is real time here; wait for the settled phase instead
    // of sleeping a fixed span, with a bound that tolerates a loaded host.
    let deadline = Instant::now() + Duration::from_secs(30);
    while state_for_view(&context, &created.record).unwrap().phase != TurnPhase::Waiting {
        assert!(Instant::now() < deadline, "the Stop never settled");
        std::thread::sleep(Duration::from_millis(50));
    }
}
