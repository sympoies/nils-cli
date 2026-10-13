use super::*;

#[tokio::test]
async fn activity_stream_settles_claude_stop_without_later_hooks_or_polling() {
    assert_claude_completion_stream(None, crate::activity::TurnPhase::Working).await;
}

#[tokio::test]
async fn activity_stream_settles_claude_stop_with_pending_attention() {
    assert_claude_completion_stream(Some("Elicitation"), crate::activity::TurnPhase::NeedsInput)
        .await;
}

#[tokio::test]
async fn activity_stream_settles_claude_stop_with_cleared_approval() {
    assert_claude_completion_stream(
        Some("PermissionRequest"),
        crate::activity::TurnPhase::Working,
    )
    .await;
}

async fn assert_claude_completion_stream(
    attention_event: Option<&str>,
    expected_phase: crate::activity::TurnPhase,
) {
    use std::os::unix::fs::PermissionsExt;
    let tmp = tempfile::TempDir::new().unwrap();
    let context = CliContext {
        state_dir: tmp.path().join("state"),
        host: None,
    };
    let record: crate::SessionRecord = serde_json::from_value(json!({
            "schema_version":"agent-session.session.v1", "id":"completion-stream",
            "agent":"claude", "mode":"interactive", "title":null,
            "cwd":tmp.path(), "tmux_session":"hs-claude-completion-stream",
            "prompt_file":null, "log_file":null,
            "created_at":jiff::Timestamp::now().to_string(), "updated_at":jiff::Timestamp::now().to_string(),
            "runtime":{"kind":"tmux", "tmux_session":"hs-claude-completion-stream",
                "generation":1, "started_at":jiff::Timestamp::now().to_string(), "launch_id":"launch-stream"}
        })).unwrap();
    let dir = context.state_dir.join("sessions").join(&record.id);
    fs::create_dir_all(&dir).unwrap();
    fs::write(
        dir.join("session.json"),
        serde_json::to_vec(&record).unwrap(),
    )
    .unwrap();
    crate::activity::activate_runtime(&context, &record).unwrap();
    let mut names = vec!["UserPromptSubmit"];
    names.extend(attention_event);
    names.push("Stop");
    for name in names {
        let mut raw = json!({"hook_event_name":name, "turn_id":"turn-1",
                "stop_hook_active":false, "background_tasks":[]});
        if name == "Elicitation" {
            raw["elicitation_id"] = json!("clarification-1");
        }
        let payload = serde_json::to_vec(&raw).unwrap();
        crate::activity::ingest_provider_hook_input(
            &context,
            crate::AgentKind::Claude,
            None,
            crate::activity::ProviderHookInput {
                id: &record.id,
                runtime_id: "launch-stream",
                payload: &payload,
                attention_authority: None,
            },
        )
        .unwrap();
        if Some(name) == attention_event {
            let state = crate::activity::state_for_view(&context, &record).unwrap();
            pretty_assertions::assert_eq!(state.phase, crate::activity::TurnPhase::NeedsInput);
            pretty_assertions::assert_eq!(
                state.current_turn.unwrap().attention.unwrap().kind,
                if name == "Elicitation" {
                    "clarification"
                } else {
                    "approval"
                }
            );
        }
    }
    assert!(crate::activity::completion_deadline_for_view(&context, &record).is_some());
    let tmux = tmp.path().join("fake-tmux");
    fs::write(&tmux, "#!/bin/sh\ncase \"$1\" in\n list-windows) printf 'hs-claude-completion-stream\\t100\\n' ;;\n display-message) printf 'Claude\\n' ;;\n capture-pane) printf '❯ \\n' ;;\nesac\n").unwrap();
    fs::set_permissions(&tmux, fs::Permissions::from_mode(0o700)).unwrap();
    let collector = default_session_collector();
    let broker = ActivityBroker::for_test_with_session_collector(
        MACHINE,
        &context,
        &tmux,
        collector.clone(),
    );
    // Keep the notification channel open but send no changes: collector side
    // effects must not accidentally act as a completion timer.
    let (_change_tx, change_rx) = mpsc::channel(1);
    let change_loop = tokio::spawn(activity_change_loop(
        broker.log.clone(),
        broker.lifecycle.clone(),
        collector,
        context,
        tmux,
        change_rx,
    ));
    broker.loops.lock().unwrap().push(change_loop);
    let mut subscription = broker.subscribe(None).unwrap();
    let first = subscription.next_event().await.unwrap();
    let initial = first.sessions.as_ref().unwrap()[0]
        .turn_state
        .as_ref()
        .unwrap();
    pretty_assertions::assert_eq!(initial.phase, expected_phase);
    let attention = initial.current_turn.as_ref().unwrap().attention.as_ref();
    if attention_event == Some("Elicitation") {
        pretty_assertions::assert_eq!(
            attention.map(|attention| attention.kind.as_str()),
            Some("clarification")
        );
    } else {
        assert!(attention.is_none(), "approval survived the same-turn Stop");
    }
    let completed = tokio::time::timeout(Duration::from_secs(3), async {
        loop {
            let event = subscription.next_event().await.unwrap();
            if event.sessions.as_ref().is_some_and(|sessions| {
                sessions.iter().any(|session| {
                    session
                        .turn_state
                        .as_ref()
                        .is_some_and(|state| state.phase == crate::activity::TurnPhase::Waiting)
                })
            }) {
                break;
            }
        }
    })
    .await;
    broker.shutdown().await;
    assert!(
        completed.is_ok(),
        "Stop settled in state_for_view but never reached stream subscribers"
    );
}
