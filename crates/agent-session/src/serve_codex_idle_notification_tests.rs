mod codex_idle_notification_tests {
    use super::*;
    use pretty_assertions::assert_eq;

    fn idle_notification_fixture() -> (tempfile::TempDir, CliContext, SessionRecord) {
        let tmp = tempfile::TempDir::new().expect("tempdir");
        seed_session_with_runtime(tmp.path(), "idle-recipient", "codex", "fixture-recipient");
        let context = CliContext {
            state_dir: tmp.path().to_path_buf(),
            host: None,
        };
        let record = load_session_record(&context, "idle-recipient").expect("record");
        crate::activity::activate_runtime(&context, &record).expect("activate runtime");
        (tmp, context, record)
    }

    fn idle_notification_event(
        context: &CliContext,
        record: &SessionRecord,
        event_id: &str,
        kind: &str,
        turn_id: &str,
        confidence: &str,
    ) -> crate::activity::ActivityResult {
        let event = serde_json::from_value(json!({
            "schema_version": crate::activity::TURN_EVENT_VERSION,
            "event_id": event_id,
            "runtime_id": record.runtime.as_ref().expect("runtime").launch_id,
            "provider": "codex",
            "provider_turn_id": turn_id,
            "kind": kind,
            "confidence": confidence
        }))
        .expect("event");
        crate::activity::ingest_event(context, &record.id, event).expect("ingest event")
    }

    fn complete_idle_notification_turn(context: &CliContext, record: &SessionRecord) {
        idle_notification_event(
            context,
            record,
            "idle-start",
            "turn_started",
            "idle-turn",
            "authoritative",
        );
        idle_notification_event(
            context,
            record,
            "idle-complete",
            "turn_completed",
            "idle-turn",
            "authoritative",
        );
        assert!(terminal_notification_waiting(
            context,
            record,
            AgentKind::Codex
        ));
    }

    #[test]
    fn codex_idle_notification_late_stop_preserves_completed_turn_eligibility() {
        let (_tmp, context, record) = idle_notification_fixture();
        complete_idle_notification_turn(&context, &record);
        idle_notification_event(
            &context,
            &record,
            "late-stop",
            "stop_observed",
            "idle-turn",
            "observed",
        );
        let duplicate = idle_notification_event(
            &context,
            &record,
            "duplicate-complete",
            "turn_completed",
            "idle-turn",
            "authoritative",
        );
        assert!(
            duplicate.duplicate,
            "replayed completion must remain deduplicated"
        );
        let projected = activity::state_for_view(&context, &record).expect("projection");
        assert_eq!(projected.phase, activity::TurnPhase::Waiting);
        assert!(projected.current_turn.is_none());
        assert_eq!(
            projected
                .last_turn
                .as_ref()
                .expect("completed turn")
                .outcome,
            "completed"
        );
        assert!(
            terminal_notification_waiting(&context, &record, AgentKind::Codex),
            "a late Stop that starts no turn must not disable wake after exact completion"
        );
    }

    #[test]
    fn codex_idle_notification_stop_alone_does_not_authorize_wake() {
        let (_tmp, context, record) = idle_notification_fixture();
        idle_notification_event(
            &context,
            &record,
            "open-start",
            "turn_started",
            "open-turn",
            "authoritative",
        );
        idle_notification_event(
            &context,
            &record,
            "open-stop",
            "stop_observed",
            "open-turn",
            "observed",
        );
        let projected = activity::state_for_view(&context, &record).expect("projection");
        assert_eq!(projected.phase, activity::TurnPhase::Working);
        assert!(projected.current_turn.is_some());
        assert!(!terminal_notification_waiting(
            &context,
            &record,
            AgentKind::Codex
        ));
    }

    #[test]
    fn codex_idle_notification_new_turn_cancels_completed_turn_eligibility() {
        let (_tmp, context, record) = idle_notification_fixture();
        complete_idle_notification_turn(&context, &record);
        idle_notification_event(
            &context,
            &record,
            "new-start",
            "turn_started",
            "new-turn",
            "authoritative",
        );
        let projected_new_turn = activity::state_for_view(&context, &record)
            .expect("new turn projection")
            .current_turn
            .expect("new turn")
            .provider_turn_id;
        assert!(projected_new_turn.is_some());
        idle_notification_event(
            &context,
            &record,
            "new-stop",
            "stop_observed",
            "new-turn",
            "observed",
        );
        let projected = activity::state_for_view(&context, &record).expect("projection");
        assert_eq!(projected.phase, activity::TurnPhase::Working);
        assert_eq!(
            projected
                .current_turn
                .as_ref()
                .expect("new turn")
                .provider_turn_id
                .as_deref(),
            projected_new_turn.as_deref()
        );
        assert!(!terminal_notification_waiting(
            &context,
            &record,
            AgentKind::Codex
        ));
    }

    #[test]
    fn codex_idle_notification_unbound_stop_cannot_inherit_completion_authority() {
        for (turn_id, source_kind) in [
            (None, "provider_hook"),
            (Some("unrelated-turn"), "provider_hook"),
            (Some("idle-turn"), "console_observation"),
            (Some("idle-turn"), "terminal_heuristic"),
        ] {
            let (_tmp, context, record) = idle_notification_fixture();
            complete_idle_notification_turn(&context, &record);
            let event = serde_json::from_value(json!({
                "schema_version": crate::activity::TURN_EVENT_VERSION,
                "event_id": "unbound-stop",
                "source_kind": source_kind,
                "runtime_id": record.runtime.as_ref().expect("runtime").launch_id,
                "provider": "codex",
                "provider_turn_id": turn_id,
                "kind": "stop_observed",
                "confidence": "observed"
            }))
            .expect("event");
            crate::activity::ingest_event(&context, &record.id, event).expect("ingest Stop");
            let projected = activity::state_for_view(&context, &record).expect("projection");
            assert_eq!(projected.source.confidence, activity::Confidence::Observed);
            assert!(!terminal_notification_waiting(
                &context,
                &record,
                AgentKind::Codex
            ));
        }
    }
}
