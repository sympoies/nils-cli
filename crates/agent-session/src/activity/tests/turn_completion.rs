use super::*;
use pretty_assertions::assert_eq;

fn ingest_turn(
    context: &CliContext,
    record: &SessionRecord,
    kind: TurnEventKind,
    turn: &str,
    id: &str,
) -> ActivityResult {
    let mut input = event(kind, id);
    input.runtime_id = record.runtime.as_ref().unwrap().launch_id.clone();
    input.provider = record.agent.clone();
    input.provider_turn_id = Some(turn.to_string());
    ingest_event(context, &record.id, input).expect("ingest turn evidence")
}

fn stopped_turn(context: &CliContext, record: &SessionRecord, turn: &str) -> TurnState {
    ingest_turn(context, record, TurnEventKind::TurnStarted, turn, "start");
    ingest_turn(context, record, TurnEventKind::Progress, turn, "progress");
    ingest_turn(context, record, TurnEventKind::StopObserved, turn, "stop").turn_state
}

#[test]
fn unmatched_completion_preserves_stop_presentation_until_accepted_work() {
    let tmp = tempfile::TempDir::new().unwrap();
    let (context, created) = test_session(&tmp);
    activate_runtime(&context, &created.record).unwrap();
    let stopped = stopped_turn(&context, &created.record, "A");
    assert_eq!(
        stopped.semantic_event.as_ref().unwrap().kind,
        "stop_observed"
    );
    assert_eq!(
        stopped.diagnostic.as_ref().unwrap().reason,
        "completion_evidence_pending"
    );
    let mut late = event(TurnEventKind::TurnCompleted, "late-B");
    late.runtime_id = created.record.runtime.as_ref().unwrap().launch_id.clone();
    late.provider_turn_id = Some("B".to_string());
    let received = ingest_event(&context, &created.record.id, late.clone()).unwrap();
    assert_eq!(received.turn_state, stopped);
    let accepted =
        read_document(&session_dir(&context, &created.record.id).join(ACTIVITY_FILE)).unwrap();
    assert_eq!(
        accepted.last_provider_event_kind,
        Some(TurnEventKind::StopObserved)
    );
    assert_eq!(accepted.last_provider_event_turn_id.as_deref(), Some("A"));
    assert_eq!(
        accepted.last_event_at.as_deref(),
        Some(
            stopped
                .semantic_event
                .as_ref()
                .unwrap()
                .observed_at
                .as_str()
        )
    );
    let duplicate = ingest_event(&context, &created.record.id, late).unwrap();
    assert!(duplicate.duplicate);
    assert_eq!(duplicate.turn_state, stopped);
    let journal =
        fs::read_to_string(session_dir(&context, &created.record.id).join(ACTIVITY_JOURNAL_FILE))
            .unwrap();
    assert!(
        journal.contains("late-B"),
        "unapplied receipt stays journaled"
    );

    let progress = ingest_turn(
        &context,
        &created.record,
        TurnEventKind::Progress,
        "A",
        "fresh-progress",
    );
    assert_eq!(progress.turn_state.phase, TurnPhase::Working);
    assert!(progress.turn_state.diagnostic.is_none());
    assert_eq!(progress.turn_state.semantic_event.unwrap().kind, "progress");
    let completed = ingest_turn(
        &context,
        &created.record,
        TurnEventKind::TurnCompleted,
        "A",
        "matching-completion",
    );
    assert_eq!(completed.turn_state.phase, TurnPhase::Waiting);
    assert!(completed.turn_state.current_turn.is_none());
}

#[test]
fn late_old_completion_preserves_newer_work_and_stop_candidate() {
    for provider in [AgentKind::Codex, AgentKind::Claude] {
        let tmp = tempfile::TempDir::new().unwrap();
        let (context, created) = test_session_for_agent(&tmp, provider);
        activate_runtime(&context, &created.record).unwrap();
        ingest_turn(
            &context,
            &created.record,
            TurnEventKind::TurnStarted,
            "A",
            "start-A",
        );
        // Starting B closes A as interrupted; a late A cannot close B.
        let working = ingest_turn(
            &context,
            &created.record,
            TurnEventKind::TurnStarted,
            "B",
            "start-B",
        )
        .turn_state;
        let late = ingest_turn(
            &context,
            &created.record,
            TurnEventKind::TurnCompleted,
            "A",
            "late-A-working",
        );
        assert_eq!(late.turn_state, working);
        let mut stop = event(TurnEventKind::StopObserved, "stop-B");
        stop.runtime_id = created.record.runtime.as_ref().unwrap().launch_id.clone();
        stop.provider = created.record.agent.clone();
        stop.provider_turn_id = Some("B".to_string());
        stop.completion_candidate = provider == AgentKind::Claude;
        let stopped = ingest_event(&context, &created.record.id, stop)
            .unwrap()
            .turn_state;
        let path = session_dir(&context, &created.record.id).join(ACTIVITY_FILE);
        let mut snapshot = read_document(&path).unwrap();
        if let Some(pending) = snapshot.pending_completion.as_mut() {
            // Hold the quiet window open so this tests rejection, not elapsed time.
            pending.received_at = "2099-01-01T00:00:00Z".to_string();
            write_document(&path, &mut snapshot).unwrap();
        }
        let pending = snapshot.pending_completion;
        let late = ingest_turn(
            &context,
            &created.record,
            TurnEventKind::TurnCompleted,
            "A",
            "late-A-stopped",
        );
        assert_eq!(late.turn_state, stopped);
        assert_eq!(
            serde_json::to_value(read_document(&path).unwrap().pending_completion).unwrap(),
            serde_json::to_value(pending).unwrap()
        );
    }
}

#[test]
fn claude_idle_after_stop_reconciles_changed_prompt_without_new_start() {
    let tmp = tempfile::TempDir::new().unwrap();
    let (context, created) = test_session_for_agent(&tmp, AgentKind::Claude);
    activate_runtime(&context, &created.record).unwrap();
    stopped_turn(&context, &created.record, "A");
    // Metadata projected from Stop(A), then idle_prompt(B) after local /compact.
    // No transcript, command content, or synthetic turn_started(B) is required.
    let completed = ingest_turn(
        &context,
        &created.record,
        TurnEventKind::TurnCompleted,
        "B",
        "idle-B",
    );
    assert_eq!(completed.turn_state.phase, TurnPhase::Waiting);
    assert!(completed.turn_state.current_turn.is_none());
    let last = completed.turn_state.last_turn.as_ref().unwrap();
    assert_eq!(last.provider_turn_id.as_deref(), Some("A"));
    assert_eq!(last.outcome, "completed");
    assert!(completed.turn_state.diagnostic.is_none());
    assert_eq!(
        state_for_view(&context, &created.record).unwrap().phase,
        TurnPhase::Waiting
    );
}

#[test]
fn claude_changed_prompt_completion_requires_stop_without_new_work_or_attention() {
    for boundary in ["no-stop", "progress", "new-start", "attention"] {
        let tmp = tempfile::TempDir::new().unwrap();
        let (context, created) = test_session_for_agent(&tmp, AgentKind::Claude);
        activate_runtime(&context, &created.record).unwrap();
        ingest_turn(
            &context,
            &created.record,
            TurnEventKind::TurnStarted,
            "A",
            "start",
        );
        if boundary != "no-stop" {
            ingest_turn(
                &context,
                &created.record,
                TurnEventKind::StopObserved,
                "A",
                "stop",
            );
        }
        match boundary {
            "progress" => {
                ingest_turn(
                    &context,
                    &created.record,
                    TurnEventKind::Progress,
                    "A",
                    "continued",
                );
            }
            "new-start" => {
                ingest_turn(
                    &context,
                    &created.record,
                    TurnEventKind::TurnStarted,
                    "C",
                    "start-C",
                );
            }
            "attention" => {
                let mut attention = event(TurnEventKind::AttentionRequested, "attention");
                attention.runtime_id = created.record.runtime.as_ref().unwrap().launch_id.clone();
                attention.provider = "claude".to_string();
                attention.provider_turn_id = Some("A".to_string());
                attention.attention_id = Some("pending".to_string());
                attention.attention_kind = Some("approval".to_string());
                ingest_event(&context, &created.record.id, attention).unwrap();
                ingest_turn(
                    &context,
                    &created.record,
                    TurnEventKind::StopObserved,
                    "A",
                    "stop-with-attention",
                );
            }
            _ => {}
        }
        let before = activity_status(&context, &created.record.id)
            .unwrap()
            .turn_state;
        let late = ingest_turn(
            &context,
            &created.record,
            TurnEventKind::TurnCompleted,
            "B",
            "idle-B",
        );
        assert_eq!(late.turn_state, before, "{boundary}");
    }
}

#[test]
fn claude_changed_prompt_completion_retains_runtime_and_conversation_fences() {
    let tmp = tempfile::TempDir::new().unwrap();
    let (context, created) = test_session_for_agent(&tmp, AgentKind::Claude);
    activate_runtime(&context, &created.record).unwrap();
    let stopped = stopped_turn(&context, &created.record, "A");
    for boundary in ["runtime", "conversation", "missing-conversation"] {
        let mut late = event(TurnEventKind::TurnCompleted, boundary);
        late.runtime_id = created.record.runtime.as_ref().unwrap().launch_id.clone();
        late.provider = "claude".to_string();
        late.provider_turn_id = Some("B".to_string());
        match boundary {
            "runtime" => late.runtime_id = "previous-runtime".to_string(),
            "conversation" => late.provider_session_id = Some("another-conversation".to_string()),
            _ => late.provider_session_id = None,
        }
        assert!(
            ingest_event(&context, &created.record.id, late).is_err(),
            "{boundary}"
        );
        assert_eq!(
            activity_status(&context, &created.record.id)
                .unwrap()
                .turn_state,
            stopped
        );
    }
}
