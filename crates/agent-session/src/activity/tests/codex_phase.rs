use super::*;
use pretty_assertions::assert_eq;

fn input(record: &SessionRecord, kind: TurnEventKind, turn: &str, key: &str) -> TurnEvent {
    let completed = kind == TurnEventKind::TurnCompleted;
    let mut event = event(kind, key);
    event.runtime_id = record.runtime.as_ref().unwrap().launch_id.clone();
    event.provider_turn_id = Some(turn.to_string());
    if completed {
        event.confidence = Confidence::Authoritative;
    }
    event
}

#[test]
fn interrupted_then_unannounced_turn_completion_returns_to_idle() {
    let tmp = tempfile::TempDir::new().unwrap();
    let (context, created) = test_session(&tmp);
    activate_runtime(&context, &created.record).unwrap();
    for (kind, turn, key) in [
        (TurnEventKind::TurnStarted, "A", "start-A"),
        (TurnEventKind::TurnStarted, "B", "start-B"),
        // A subsequent turn has progress/Stop but its start hook was lost.
        (TurnEventKind::Progress, "C", "progress-C"),
        (TurnEventKind::StopObserved, "C", "stop-C"),
    ] {
        ingest_event(
            &context,
            &created.record.id,
            input(&created.record, kind, turn, key),
        )
        .unwrap();
    }
    let before = activity_status(&context, &created.record.id)
        .unwrap()
        .turn_state;
    assert_eq!(before.last_turn.unwrap().outcome, "interrupted");
    let completed = ingest_event(
        &context,
        &created.record.id,
        input(
            &created.record,
            TurnEventKind::TurnCompleted,
            "C",
            "complete-C",
        ),
    )
    .unwrap()
    .turn_state;
    assert_eq!(completed.phase, TurnPhase::Waiting);
    assert!(completed.current_turn.is_none());
    assert_eq!(completed.last_turn.unwrap().outcome, "completed");
}

#[test]
fn quiet_projection_keeps_a_codex_turn_out_of_the_unknown_attention_state() {
    let tmp = tempfile::TempDir::new().unwrap();
    let (context, created) = test_session(&tmp);
    activate_runtime(&context, &created.record).unwrap();
    ingest_event(
        &context,
        &created.record.id,
        input(&created.record, TurnEventKind::TurnStarted, "A", "start-A"),
    )
    .unwrap();
    let path = session_dir(&context, &created.record.id).join(ACTIVITY_FILE);
    let mut doc = read_document(&path).unwrap();
    doc.state.phase_changed_at = "2000-01-01T00:00:00Z".into();
    let current = doc.state.current_turn.as_mut().unwrap();
    current.started_at = "2000-01-01T00:00:00Z".into();
    current.last_progress_at = Some("2000-01-01T00:01:00Z".into());
    write_document(&path, &mut doc).unwrap();
    let raw = fs::read(&path).unwrap();
    let first = state_for_view(&context, &created.record).unwrap();
    assert_eq!(first.phase, TurnPhase::Working);
    assert_eq!(first.phase_changed_at, "2000-01-01T00:00:00Z");
    assert_eq!(first.source.confidence, Confidence::Inferred);
    assert!(codex_notification_maybe_idle(&first));
    assert_eq!(
        controller_state_for_record(&context, &created.record)
            .unwrap()
            .phase,
        TurnPhase::Working
    );
    assert_eq!(
        serde_json::to_value(&first).unwrap()["diagnostic"]["quiet_threshold_seconds"],
        300
    );
    assert_eq!(state_for_view(&context, &created.record).unwrap(), first);
    assert_eq!(
        fs::read(&path).unwrap(),
        raw,
        "projection does not falsify stored provider history"
    );
}

#[test]
fn recent_progress_keeps_working_even_if_phase_started_hours_ago() {
    let tmp = tempfile::TempDir::new().unwrap();
    let (context, created) = test_session(&tmp);
    activate_runtime(&context, &created.record).unwrap();
    ingest_event(
        &context,
        &created.record.id,
        input(&created.record, TurnEventKind::TurnStarted, "A", "start-A"),
    )
    .unwrap();
    let path = session_dir(&context, &created.record.id).join(ACTIVITY_FILE);
    let mut doc = read_document(&path).unwrap();
    doc.state.phase_changed_at = "2000-01-01T00:00:00Z".into();
    doc.state.current_turn.as_mut().unwrap().last_progress_at = Some(now());
    write_document(&path, &mut doc).unwrap();
    assert_eq!(
        state_for_view(&context, &created.record).unwrap().phase,
        TurnPhase::Working
    );
}

#[test]
fn already_stuck_completion_receipt_heals_on_first_read_without_new_event() {
    let tmp = tempfile::TempDir::new().unwrap();
    let (context, created) = test_session(&tmp);
    activate_runtime(&context, &created.record).unwrap();
    for (kind, turn, key) in [
        (TurnEventKind::TurnStarted, "A", "start-A"),
        (TurnEventKind::TurnStarted, "B", "start-B"),
        (TurnEventKind::Progress, "C", "progress-C"),
        (TurnEventKind::StopObserved, "C", "stop-C"),
    ] {
        ingest_event(
            &context,
            &created.record.id,
            input(&created.record, kind, turn, key),
        )
        .unwrap();
    }
    let path = session_dir(&context, &created.record.id).join(ACTIVITY_FILE);
    let old = read_document(&path).unwrap();
    ingest_event(
        &context,
        &created.record.id,
        input(
            &created.record,
            TurnEventKind::TurnCompleted,
            "C",
            "complete-C",
        ),
    )
    .unwrap();
    let mut old_projection = read_document(&path).unwrap();
    // Preserve the committed receipt/replay counters, but restore the old
    // projection that left B open despite C's progress and completion.
    old_projection.state = old.state;
    old_projection.last_provider_event_kind = old.last_provider_event_kind;
    old_projection.last_provider_event_at = old.last_provider_event_at;
    old_projection.last_provider_event_turn_id = old.last_provider_event_turn_id;
    old_projection.last_semantic_event = old.last_semantic_event;
    old_projection.last_semantic_event_at = old.last_semantic_event_at;
    old_projection.last_event_at = old.last_event_at;
    write_document(&path, &mut old_projection).unwrap();
    let raw = fs::read(&path).unwrap();
    let healed = state_for_view(&context, &created.record).unwrap();
    assert_eq!(healed.phase, TurnPhase::Waiting);
    assert!(healed.current_turn.is_none());
    assert_eq!(healed.last_turn.unwrap().outcome, "completed");
    assert_eq!(fs::read(&path).unwrap(), raw);
}

#[test]
fn older_completion_cannot_close_a_newer_progress_turn() {
    let tmp = tempfile::TempDir::new().unwrap();
    let (context, created) = test_session(&tmp);
    activate_runtime(&context, &created.record).unwrap();
    for (kind, turn, key) in [
        (TurnEventKind::TurnStarted, "A", "start-A"),
        (TurnEventKind::TurnStarted, "B", "start-B"),
        (TurnEventKind::Progress, "C", "progress-C"),
        (TurnEventKind::Progress, "D", "progress-D"),
    ] {
        ingest_event(
            &context,
            &created.record.id,
            input(&created.record, kind, turn, key),
        )
        .unwrap();
    }
    let before = activity_status(&context, &created.record.id)
        .unwrap()
        .turn_state;
    let late = ingest_event(
        &context,
        &created.record.id,
        input(&created.record, TurnEventKind::TurnCompleted, "C", "late-C"),
    )
    .unwrap();
    assert_eq!(late.turn_state, before);
}

#[test]
fn quiet_hint_never_clears_attention_or_claims_idle_completion() {
    let tmp = tempfile::TempDir::new().unwrap();
    let (context, created) = test_session(&tmp);
    activate_runtime(&context, &created.record).unwrap();
    ingest_event(
        &context,
        &created.record.id,
        input(&created.record, TurnEventKind::TurnStarted, "A", "start-A"),
    )
    .unwrap();
    let mut attention = input(
        &created.record,
        TurnEventKind::AttentionRequested,
        "A",
        "approval",
    );
    attention.attention_id = Some("approval-1".into());
    attention.attention_kind = Some("approval".into());
    ingest_event(&context, &created.record.id, attention).unwrap();
    let path = session_dir(&context, &created.record.id).join(ACTIVITY_FILE);
    let mut doc = read_document(&path).unwrap();
    doc.state.current_turn.as_mut().unwrap().last_progress_at = Some("2000-01-01T00:00:00Z".into());
    write_document(&path, &mut doc).unwrap();
    let view = state_for_view(&context, &created.record).unwrap();
    assert_eq!(view.phase, TurnPhase::NeedsInput);
    assert!(view.current_turn.unwrap().attention.is_some());
}

#[test]
fn nonprovider_progress_cannot_extend_the_provider_quiet_clock() {
    let tmp = tempfile::TempDir::new().unwrap();
    let (context, created) = test_session(&tmp);
    activate_runtime(&context, &created.record).unwrap();
    ingest_event(
        &context,
        &created.record.id,
        input(&created.record, TurnEventKind::TurnStarted, "A", "start-A"),
    )
    .unwrap();
    let path = session_dir(&context, &created.record.id).join(ACTIVITY_FILE);
    let mut doc = read_document(&path).unwrap();
    let current = doc.state.current_turn.as_mut().unwrap();
    current.started_at = "2000-01-01T00:00:00Z".into();
    current.last_progress_at = Some("2000-01-01T00:01:00Z".into());
    write_document(&path, &mut doc).unwrap();
    let mut observation = input(
        &created.record,
        TurnEventKind::Progress,
        "A",
        "runtime-progress",
    );
    observation.source_kind = SourceKind::Runtime;
    ingest_event(&context, &created.record.id, observation).unwrap();
    let view = state_for_view(&context, &created.record).unwrap();
    assert_eq!(view.phase, TurnPhase::Working);
    assert_eq!(view.source.confidence, Confidence::Inferred);
    assert!(codex_notification_maybe_idle(&view));
    assert_eq!(
        view.current_turn.unwrap().last_progress_at.as_deref(),
        Some("2000-01-01T00:01:00Z")
    );
}

#[test]
fn never_turned_codex_is_a_notification_probe_candidate() {
    let tmp = tempfile::TempDir::new().unwrap();
    let (context, created) = test_session(&tmp);
    activate_runtime(&context, &created.record).unwrap();
    let state = state_for_view(&context, &created.record).unwrap();
    assert_eq!(state.phase, TurnPhase::Starting);
    assert!(state.current_turn.is_none());
    assert!(state.last_turn.is_none());
    assert!(codex_notification_maybe_idle(&state));
}
