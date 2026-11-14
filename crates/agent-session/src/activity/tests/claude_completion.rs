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
fn claude_stop_qualifiers_and_failure_remain_structured() {
    for (event_name, stop_fields, expected) in [
        (
            "Stop",
            json!({"stop_hook_active": false, "background_tasks": []}),
            TurnPhase::Working,
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
fn elapsed_time_alone_cannot_complete_a_claude_stop() {
    let tmp = tempfile::TempDir::new().unwrap();
    let clock = TestClock::freeze();
    let (context, created) = test_session_for_agent(&tmp, AgentKind::Claude);
    activate_runtime(&context, &created.record).unwrap();
    let runtime = &created.record.runtime.as_ref().unwrap().launch_id;
    for name in ["UserPromptSubmit", "Stop"] {
        let raw = json!({"hook_event_name": name, "session_id": "session-1",
            "turn_id": "turn-1", "stop_hook_active": false, "background_tasks": []});
        let event = normalize_provider_hook(AgentKind::Claude, None, runtime, &raw)
            .unwrap()
            .unwrap();
        ingest_event(&context, &created.record.id, event).unwrap();
    }
    clock.advance(Duration::from_secs(60));
    let state = state_for_view(&context, &created.record).unwrap();
    assert_eq!(
        state.phase,
        TurnPhase::Working,
        "a Stop needs structured idle confirmation"
    );
    assert!(state.current_turn.is_some());
    assert!(state.last_turn.is_none());
}

#[test]
fn qualifying_stop_requires_a_fresh_runtime_and_revision_bound_structured_idle() {
    for case in [
        "current",
        "busy",
        "blocked",
        "unavailable",
        "old-revision",
        "old-runtime",
        "old-generation",
        "stale",
        "before-stop",
        "terminal-screen",
        "continued",
        "parallel-stop",
        "settled-history",
    ] {
        let tmp = tempfile::TempDir::new().unwrap();
        let _clock = TestClock::freeze();
        let (context, created) = test_session_for_agent(&tmp, AgentKind::Claude);
        activate_runtime(&context, &created.record).unwrap();
        let runtime = created.record.runtime.as_ref().unwrap();
        for name in ["UserPromptSubmit", "Stop"] {
            let raw = json!({"hook_event_name": name, "session_id": "session-1",
                "turn_id": "turn-1", "stop_hook_active": false, "background_tasks": []});
            let event = normalize_provider_hook(AgentKind::Claude, None, &runtime.launch_id, &raw)
                .unwrap()
                .unwrap();
            ingest_event(&context, &created.record.id, event).unwrap();
        }
        let before = state_for_view(&context, &created.record).unwrap();
        let mut sidecar = json!({
            "schema_version": "agent-session.activity-shadow.v1",
            "runtime_id": runtime.launch_id, "runtime_generation": runtime.generation,
            "activity_revision": before.revision,
            "observation": {"observer_version": "claude-agents.v1",
                "rule_id": "claude_agents_idle", "observed_at": now(),
                "projection": "waiting", "disagrees": true}
        });
        if case == "settled-history" {
            // The idle reading followed Stop, but both are older than a poll.
            // With no newer accepted activity, completion cannot reopen.
            let path = session_dir(&context, &created.record.id).join(ACTIVITY_FILE);
            let mut snapshot = read_document(&path).unwrap();
            snapshot.pending_completion.as_mut().unwrap().received_at = jiff::Timestamp::now()
                .checked_sub(jiff::SignedDuration::from_secs(60))
                .unwrap()
                .to_string();
            fs::write(path, serde_json::to_vec(&snapshot).unwrap()).unwrap();
            sidecar["observation"]["observed_at"] = json!(
                jiff::Timestamp::now()
                    .checked_sub(jiff::SignedDuration::from_secs(30))
                    .unwrap()
                    .to_string()
            );
        }
        match case {
            "busy" => {
                sidecar["observation"]["rule_id"] = json!("claude_agents_busy");
                sidecar["observation"]["projection"] = json!("working");
            }
            "blocked" => {
                sidecar["observation"]["rule_id"] =
                    json!("claude_agents_waiting_permission_prompt");
                sidecar["observation"]["projection"] = json!("needs_input");
            }
            "unavailable" => {
                sidecar["observation"]["rule_id"] = json!("claude_agents_unavailable");
                sidecar["observation"]["projection"] = json!("unknown");
            }
            "old-revision" => sidecar["activity_revision"] = json!(before.revision - 1),
            "old-runtime" => sidecar["runtime_id"] = json!("old-runtime"),
            "old-generation" => sidecar["runtime_generation"] = json!(runtime.generation + 1),
            "stale" => sidecar["observation"]["observed_at"] = json!("2000-01-01T00:00:00Z"),
            "before-stop" => {
                sidecar["observation"]["observed_at"] = json!(
                    jiff::Timestamp::now()
                        .checked_sub(jiff::SignedDuration::from_secs(1))
                        .unwrap()
                        .to_string()
                )
            }
            "terminal-screen" => {
                sidecar["observation"]["observer_version"] = json!("terminal-shadow.v1");
                sidecar["observation"]["rule_id"] = json!("claude_prompt_visible");
            }
            _ => {}
        }
        fs::write(
            session_dir(&context, &created.record.id).join("activity.shadow.json"),
            serde_json::to_vec(&sidecar).unwrap(),
        )
        .unwrap();
        if matches!(case, "continued" | "parallel-stop") {
            let input: TurnEvent = serde_json::from_value(json!({
                "schema_version":"agent-session.turn-event.v1", "event_id":case,
                "runtime_id":runtime.launch_id, "provider":"claude",
                "provider_turn_id":"turn-1", "provider_session_id":"session-1",
                "kind":if case == "parallel-stop" { "stop_observed" } else { "progress" },
                "confidence":"observed", "source_kind":"provider_hook"
            }))
            .unwrap();
            ingest_event(&context, &created.record.id, input).unwrap();
        }
        let after = state_for_view(&context, &created.record).unwrap();
        assert_eq!(
            after.phase,
            if matches!(case, "current" | "settled-history") {
                TurnPhase::Waiting
            } else if matches!(case, "unavailable" | "before-stop") {
                TurnPhase::Unknown
            } else {
                TurnPhase::Working
            },
            "{case}"
        );
        if case == "before-stop" {
            assert_eq!(
                after.diagnostic.as_ref().unwrap().reason,
                "claude_turn_outcome_unknown"
            );
        }
        if matches!(case, "current" | "settled-history") {
            assert!(after.current_turn.is_none());
            assert_eq!(after.last_turn.unwrap().outcome, "completed");
        } else if case != "continued" {
            assert_eq!(after.current_turn, before.current_turn);
            assert_eq!(after.last_turn, before.last_turn);
        }
    }
}
