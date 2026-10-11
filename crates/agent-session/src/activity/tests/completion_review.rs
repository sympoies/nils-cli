use super::*;
use pretty_assertions::assert_eq;

fn claude_hook(
    context: &CliContext,
    record: &SessionRecord,
    turn: &str,
    name: &str,
    fields: Value,
) -> ActivityResult {
    let mut raw = json!({
        "hook_event_name": name, "session_id": "session-1", "turn_id": turn
    });
    raw.as_object_mut()
        .unwrap()
        .extend(fields.as_object().unwrap().clone());
    let input = normalize_provider_hook(
        AgentKind::Claude,
        None,
        &record.runtime.as_ref().unwrap().launch_id,
        &raw,
    )
    .unwrap()
    .unwrap();
    ingest_event(context, &record.id, input).unwrap()
}

fn hold_completion_candidate(context: &CliContext, record: &SessionRecord) {
    let path = session_dir(context, &record.id).join(ACTIVITY_FILE);
    let mut document = read_document(&path).unwrap();
    document.pending_completion.as_mut().unwrap().received_at = "2099-01-01T00:00:00Z".to_string();
    write_document(&path, &mut document).unwrap();
}

fn nonfinal_stop_does_not_reconcile(fields: Value, reentrant: bool) {
    let tmp = tempfile::TempDir::new().unwrap();
    let (context, created) = test_session_for_agent(&tmp, AgentKind::Claude);
    activate_runtime(&context, &created.record).unwrap();
    claude_hook(
        &context,
        &created.record,
        "A",
        "UserPromptSubmit",
        json!({}),
    );
    if reentrant {
        claude_hook(
            &context,
            &created.record,
            "A",
            "Stop",
            json!({"stop_hook_active": false, "background_tasks": []}),
        );
        hold_completion_candidate(&context, &created.record);
    }
    let stopped = claude_hook(&context, &created.record, "A", "Stop", fields).turn_state;
    let document =
        read_document(&session_dir(&context, &created.record.id).join(ACTIVITY_FILE)).unwrap();
    assert!(document.pending_completion.is_none());
    let idle = claude_hook(
        &context,
        &created.record,
        "B",
        "Notification",
        json!({"notification_type": "idle_prompt"}),
    );
    assert_eq!(
        idle.turn_state, stopped,
        "non-final Stop must not authorize changed-ID idle"
    );
}

#[test]
fn active_stop_rejects_changed_prompt_idle() {
    nonfinal_stop_does_not_reconcile(
        json!({"stop_hook_active": true, "background_tasks": []}),
        false,
    );
}

#[test]
fn background_stop_rejects_changed_prompt_idle() {
    nonfinal_stop_does_not_reconcile(
        json!({"stop_hook_active": false, "background_tasks": [{"id": "synthetic-task"}]}),
        false,
    );
}

#[test]
fn reentrant_stop_rejects_changed_prompt_idle() {
    nonfinal_stop_does_not_reconcile(
        json!({"stop_hook_active": true, "background_tasks": []}),
        true,
    );
}

fn rejected_failure_replay_does_not_arm(
    failure_kind: crate::codex_app_server::StructuredFailureKind,
) {
    let tmp = tempfile::TempDir::new().unwrap();
    let (context, mut created) = test_session_for_agent(&tmp, AgentKind::Codex);
    let runtime = created.record.runtime.as_mut().unwrap();
    runtime.kind = crate::codex_app_server::RUNTIME_KIND.to_string();
    runtime.extra.insert(
        crate::codex_app_server::PROTOCOL_KEY.to_string(),
        json!(crate::codex_app_server::PROTOCOL_VERSION),
    );
    for (key, file) in [
        (crate::codex_app_server::SOCKET_KEY, "socket"),
        (crate::codex_app_server::PROXY_KEY, "proxy"),
        (crate::codex_app_server::THREAD_HANDOFF_KEY, "handoff"),
        (crate::codex_app_server::THREAD_ATTACHED_KEY, "attached"),
    ] {
        runtime
            .extra
            .insert(key.to_string(), json!(tmp.path().join(file)));
    }
    crate::write_session_record(&context, &created.record).unwrap();
    activate_runtime(&context, &created.record).unwrap();
    let runtime_id = created.record.runtime.as_ref().unwrap().launch_id.clone();
    let mut start = event(TurnEventKind::TurnStarted, "start-A");
    start.runtime_id = runtime_id.clone();
    start.provider_session_id = Some("session-1".to_string());
    start.provider_turn_id = Some("A".to_string());
    let working = ingest_event(&context, &created.record.id, start)
        .unwrap()
        .turn_state;
    let enabled =
        crate::auto_resume::set_enabled(&context, &created.record.id, true, "2030-01-01T00:00:00Z")
            .unwrap();
    assert_eq!(enabled.state, "enabled");
    let path = session_dir(&context, &created.record.id).join("auto-resume.json");
    let before = fs::read(&path).unwrap();
    let failure = || {
        ingest_codex_app_server_failure_with_kind(
            &context,
            &created.record.id,
            &runtime_id,
            "session-1",
            "B",
            failure_kind,
        )
        .unwrap()
    };
    assert_eq!(failure().turn_state, working);
    assert_eq!(
        fs::read(&path).unwrap(),
        before,
        "fresh rejected receipt cannot arm"
    );
    let replay = failure();
    assert!(replay.duplicate);
    assert_eq!(replay.turn_state, working);
    assert_eq!(
        fs::read(&path).unwrap(),
        before,
        "rejected replay cannot arm"
    );

    // Matching failures still authorize recovery, including an admitted retry.
    let accepted = || {
        ingest_codex_app_server_failure_with_kind(
            &context,
            &created.record.id,
            &runtime_id,
            "session-1",
            "A",
            failure_kind,
        )
        .unwrap()
    };
    assert_eq!(accepted().turn_state.phase, TurnPhase::Waiting);
    let armed = crate::auto_resume::view_for_record(&context, &created.record);
    assert!(matches!(armed.state.as_str(), "armed" | "scheduled"));
    crate::auto_resume::set_enabled(&context, &created.record.id, true, "2030-01-01T00:00:00Z")
        .unwrap();
    assert!(accepted().duplicate);
    assert_eq!(
        crate::auto_resume::view_for_record(&context, &created.record).state,
        armed.state
    );
}

#[test]
fn rejected_usage_exhaustion_replay_does_not_arm_recovery() {
    rejected_failure_replay_does_not_arm(
        crate::codex_app_server::StructuredFailureKind::UsageExhausted,
    );
}

#[test]
fn rejected_provider_capacity_replay_does_not_arm_recovery() {
    rejected_failure_replay_does_not_arm(
        crate::codex_app_server::StructuredFailureKind::ProviderCapacity,
    );
}

#[test]
fn older_retained_idle_rejected_after_three_turns_but_unseen_compact_id_completes() {
    let tmp = tempfile::TempDir::new().unwrap();
    let (context, created) = test_session_for_agent(&tmp, AgentKind::Claude);
    activate_runtime(&context, &created.record).unwrap();
    for turn in ["X", "A", "B"] {
        claude_hook(
            &context,
            &created.record,
            turn,
            "UserPromptSubmit",
            json!({}),
        );
    }
    let stopped = claude_hook(
        &context,
        &created.record,
        "B",
        "Stop",
        json!({"stop_hook_active": false, "background_tasks": []}),
    )
    .turn_state;
    hold_completion_candidate(&context, &created.record);
    let last = stopped.last_turn.as_ref().unwrap().provider_turn_id.clone();
    assert_eq!(
        last,
        Some(
            projected_provider_identifier(
                &created.record.runtime.as_ref().unwrap().launch_id,
                AgentKind::Claude,
                "turn",
                "A"
            )
            .unwrap()
        )
    );
    let late = claude_hook(
        &context,
        &created.record,
        "X",
        "Notification",
        json!({"notification_type": "idle_prompt"}),
    );
    assert_eq!(
        late.turn_state, stopped,
        "X remains known in retained journal history"
    );
    let idle = claude_hook(
        &context,
        &created.record,
        "unseen-compact",
        "Notification",
        json!({"notification_type": "idle_prompt"}),
    );
    assert_eq!(idle.turn_state.phase, TurnPhase::Waiting);
    assert_eq!(
        idle.turn_state.last_turn.unwrap().provider_turn_id,
        stopped.current_turn.unwrap().provider_turn_id
    );
}
