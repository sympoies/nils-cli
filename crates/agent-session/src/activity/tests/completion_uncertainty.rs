use super::test_clock::TestClock;
use super::*;
use pretty_assertions::assert_eq;

#[test]
fn missing_completion_has_uniform_read_only_uncertainty() {
    for agent in [AgentKind::Codex, AgentKind::Claude, AgentKind::Dsh] {
        let tmp = tempfile::tempdir().unwrap();
        let clock = TestClock::freeze();
        let (context, mut created) = test_session_for_agent(&tmp, agent);
        if agent == AgentKind::Dsh {
            created.record.agent_bin =
                Some(tmp.path().join("fixture-launcher").display().to_string());
            created
                .record
                .runtime
                .as_mut()
                .unwrap()
                .extra
                .insert("agent_profile".into(), json!("fixture"));
            write_session_record(&context, &created.record).unwrap();
        }
        activate_runtime(&context, &created.record).unwrap();
        for (kind, key) in [
            (TurnEventKind::TurnStarted, "start"),
            (TurnEventKind::StopObserved, "stop"),
        ] {
            let mut input = event(kind, key);
            input.provider = agent.as_str().into();
            input.runtime_id = created.record.runtime.as_ref().unwrap().launch_id.clone();
            ingest_event(&context, &created.record.id, input).unwrap();
        }
        let dir = session_dir(&context, &created.record.id);
        let before = fs::read(dir.join(ACTIVITY_FILE)).unwrap();
        let journal = fs::read(dir.join(ACTIVITY_JOURNAL_FILE)).unwrap();
        clock.advance(Duration::from_secs(299));
        assert_eq!(
            state_for_view(&context, &created.record).unwrap().phase,
            TurnPhase::Working
        );
        clock.advance(Duration::from_secs(1));
        let view = state_for_view(&context, &created.record).unwrap();
        assert_eq!(view.phase, TurnPhase::Unknown, "{}", agent.as_str());
        assert_eq!(view.source.confidence, Confidence::Inferred);
        assert!(view.current_turn.is_some());
        assert!(view.last_turn.is_none());
        assert!(!codex_notification_maybe_idle(&view));
        let full = serde_json::to_value(&view).unwrap();
        let stream = serde_json::to_value(stream_projection(&view)).unwrap();
        let board = diagnostic_for_board(full["diagnostic"].as_object().unwrap()).unwrap();
        for diagnostic in [&full["diagnostic"], &stream["diagnostic"], &board] {
            assert_eq!(diagnostic["reason"], "completion_evidence_missing");
            assert_eq!(diagnostic["quiet_threshold_seconds"], 300);
        }
        assert_eq!(
            activity_status(&context, &created.record.id)
                .unwrap()
                .turn_state,
            view
        );
        assert_eq!(
            controller_state_for_record(&context, &created.record)
                .unwrap()
                .phase,
            TurnPhase::Working
        );
        assert_eq!(fs::read(dir.join(ACTIVITY_FILE)).unwrap(), before);
        assert_eq!(fs::read(dir.join(ACTIVITY_JOURNAL_FILE)).unwrap(), journal);
    }
}

#[test]
fn claude_stop_diagnoses_safe_candidate_rejection_fields() {
    for (fields, reason) in [
        (json!({}), "stop_hook_active_missing"),
        (
            json!({"stop_hook_active": "invalid-canary"}),
            "stop_hook_active_invalid",
        ),
        (
            json!({"stop_hook_active": true, "background_tasks": []}),
            "stop_hook_active",
        ),
        (
            json!({"stop_hook_active": false}),
            "background_tasks_missing",
        ),
        (
            json!({"stop_hook_active": false, "background_tasks": null}),
            "background_tasks_invalid",
        ),
        (
            json!({"stop_hook_active": false, "background_tasks": [{"description": "private-task-canary"}]}),
            "background_tasks_present",
        ),
        (
            json!({"stop_hook_active": false, "background_tasks": []}),
            "candidate",
        ),
    ] {
        let tmp = tempfile::tempdir().unwrap();
        let clock = TestClock::freeze();
        let (context, created) = test_session_for_agent(&tmp, AgentKind::Claude);
        activate_runtime(&context, &created.record).unwrap();
        let runtime_id = &created.record.runtime.as_ref().unwrap().launch_id;
        for name in ["UserPromptSubmit", "Stop"] {
            let mut raw = fields.clone();
            raw["hook_event_name"] = json!(name);
            raw["session_id"] = json!("session-1");
            raw["prompt_id"] = json!("turn-1");
            raw["last_assistant_message"] = json!("private-message-canary");
            let input = normalize_provider_hook(AgentKind::Claude, None, runtime_id, &raw)
                .unwrap()
                .unwrap();
            ingest_event(&context, &created.record.id, input).unwrap();
        }
        let state = state_for_view(&context, &created.record).unwrap();
        let full = serde_json::to_value(&state).unwrap();
        assert_eq!(full["diagnostic"]["completion_candidate_reason"], reason);
        let stream = serde_json::to_value(stream_projection(&state)).unwrap();
        let board = diagnostic_for_board(full["diagnostic"].as_object().unwrap()).unwrap();
        assert_eq!(stream["diagnostic"]["completion_candidate_reason"], reason);
        assert_eq!(board["completion_candidate_reason"], reason);
        clock.advance(Duration::from_secs(301));
        let later = state_for_view(&context, &created.record).unwrap();
        assert_eq!(
            later.phase,
            if reason == "candidate" {
                TurnPhase::Waiting
            } else {
                TurnPhase::Unknown
            }
        );
        if reason != "candidate" {
            assert!(
                later.last_turn.is_none(),
                "continuation and background work cannot complete"
            );
        }
        let persisted =
            fs::read_to_string(session_dir(&context, &created.record.id).join(ACTIVITY_FILE))
                .unwrap();
        assert!(!persisted.contains("private-task-canary"));
        assert!(!persisted.contains("private-message-canary"));
    }
}

#[test]
fn latest_stop_starts_the_window_and_fresh_evidence_restores_state() {
    for agent in [AgentKind::Codex, AgentKind::Claude] {
        let tmp = tempfile::tempdir().unwrap();
        let clock = TestClock::freeze();
        let (context, created) = test_session_for_agent(&tmp, agent);
        activate_runtime(&context, &created.record).unwrap();
        let input = |kind, key: &str| {
            let mut input = event(kind, key);
            input.provider = agent.as_str().into();
            input.runtime_id = created.record.runtime.as_ref().unwrap().launch_id.clone();
            input
        };
        ingest_event(
            &context,
            &created.record.id,
            input(TurnEventKind::TurnStarted, "start"),
        )
        .unwrap();
        clock.advance(Duration::from_secs(600));
        assert_eq!(
            state_for_view(&context, &created.record).unwrap().phase,
            TurnPhase::Working
        );
        ingest_event(
            &context,
            &created.record.id,
            input(TurnEventKind::StopObserved, "stop"),
        )
        .unwrap();
        let stopped = state_for_view(&context, &created.record).unwrap();
        assert_eq!(
            stopped.diagnostic.as_ref().unwrap().reason,
            "completion_evidence_pending"
        );
        assert!(!codex_notification_maybe_idle(&stopped));
        assert_eq!(
            state_for_view(&context, &created.record).unwrap().phase,
            TurnPhase::Working
        );
        clock.advance(Duration::from_secs(300));
        assert_eq!(
            state_for_view(&context, &created.record).unwrap().phase,
            TurnPhase::Unknown
        );
        ingest_event(
            &context,
            &created.record.id,
            input(TurnEventKind::Progress, "continued"),
        )
        .unwrap();
        clock.advance(Duration::from_secs(600));
        assert_eq!(
            state_for_view(&context, &created.record).unwrap().phase,
            TurnPhase::Working
        );
        ingest_event(
            &context,
            &created.record.id,
            input(TurnEventKind::TurnCompleted, "completed"),
        )
        .unwrap();
        assert_eq!(
            state_for_view(&context, &created.record).unwrap().phase,
            TurnPhase::Waiting
        );
    }
}

#[test]
fn stop_uncertainty_never_overrides_pending_attention() {
    for agent in [AgentKind::Codex, AgentKind::Claude] {
        let tmp = tempfile::tempdir().unwrap();
        let clock = TestClock::freeze();
        let (context, created) = test_session_for_agent(&tmp, agent);
        activate_runtime(&context, &created.record).unwrap();
        for (kind, key) in [
            (TurnEventKind::TurnStarted, "start"),
            (TurnEventKind::AttentionRequested, "question"),
            (TurnEventKind::StopObserved, "stop"),
        ] {
            let mut input = event(kind, key);
            input.provider = agent.as_str().into();
            input.runtime_id = created.record.runtime.as_ref().unwrap().launch_id.clone();
            if input.kind == TurnEventKind::AttentionRequested {
                input.attention_id = Some("pending-question".into());
                input.attention_kind = Some("clarification".into());
            }
            ingest_event(&context, &created.record.id, input).unwrap();
        }
        clock.advance(Duration::from_secs(600));
        let view = state_for_view(&context, &created.record).unwrap();
        assert_eq!(view.phase, TurnPhase::NeedsInput);
        assert_eq!(
            view.current_turn.unwrap().attention.unwrap().pending_count,
            1
        );
    }
}

#[test]
fn current_codex_attention_protocol_is_audited_without_open_ended_version_trust() {
    assert!(crate::codex_app_server::exact_attention_version_is_audited(
        (0, 162, 1)
    ));
    assert!(!crate::codex_app_server::exact_attention_version_is_audited((0, 162, 2)));
    assert!(!crate::codex_app_server::exact_attention_version_is_audited((9, 0, 0)));
}
