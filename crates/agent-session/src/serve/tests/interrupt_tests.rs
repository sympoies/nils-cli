use super::*;
use crate::codex_app_server::tests::control_fixture;
use futures_util::{SinkExt, StreamExt};
use pretty_assertions::assert_eq;

const THREAD: &str = "interrupt-thread";
const TURN: &str = "active-turn";
const URI: &str = "/sessions/interrupt/interrupt";

#[derive(Clone, Copy)]
enum TurnScenario {
    Interrupted,
    CompletedRace,
    Failed,
    Idle,
    FreshIdle,
    Unsupported,
}

impl TurnScenario {
    fn outcome(self) -> Option<&'static str> {
        match self {
            Self::Interrupted => Some("interrupted"),
            Self::CompletedRace => Some("completed"),
            Self::Failed => Some("failed"),
            Self::Idle | Self::FreshIdle => Some("no_active_turn"),
            Self::Unsupported => None,
        }
    }
}

fn interrupt_round_trip(scenario: TurnScenario) {
    control_fixture::run_short(async |root| exercise_interrupt_round_trip(root, scenario).await);
}

async fn exercise_interrupt_round_trip(root: &Path, scenario: TurnScenario) {
    let launch = seed_codex_app_server_session(root, "interrupt");
    let state = state(root, Some(TOKEN), minimal_tmux(root));
    let record = load_session_record(&state.context, "interrupt").unwrap();
    let id = record.id.clone();
    codex_app_server::replace_thread_binding(&record, THREAD).unwrap();
    activity::activate_runtime(&state.context, &record).unwrap();
    let listener = tokio::net::UnixListener::bind(root.join("interrupt.sock")).unwrap();
    let peer = tokio::spawn(async move {
        let (stream, _) = listener.accept().await.unwrap();
        let mut socket = tokio_tungstenite::accept_async(stream).await.unwrap();
        let mut queried = false;
        let mut interrupted = false;
        while let Some(Ok(ClientMessage::Text(text))) = socket.next().await {
            let request: Value = serde_json::from_str(&text).unwrap();
            let Some(id) = request.get("id") else {
                continue;
            };
            let mut response = json!({"id": id});
            response["result"] = match request["method"].as_str().unwrap() {
                "initialize" | "thread/resume" => json!({}),
                "thread/loaded/list" => json!({"data": [THREAD]}),
                "account/rateLimits/read" => json!({"rateLimits": {}}),
                "thread/read" => {
                    assert!(matches!(scenario, TurnScenario::FreshIdle) && queried);
                    json!({"thread": {"id": THREAD, "status": {"type": "idle"}}})
                }
                "thread/turns/list" => {
                    assert_eq!(request["params"]["threadId"], THREAD);
                    assert_eq!(request["params"]["limit"], 1);
                    if matches!(scenario, TurnScenario::FreshIdle) {
                        assert!(!queried);
                        queried = true;
                        let message = format!(
                            "thread {THREAD} is not materialized yet; thread/turns/list is unavailable before first user message"
                        );
                        let error = json!({"id":id, "error":{"code":-32600,"message":message}});
                        socket
                            .send(ClientMessage::Text(error.to_string().into()))
                            .await
                            .unwrap();
                        continue;
                    }
                    let status = if queried {
                        assert!(interrupted);
                        match scenario {
                            TurnScenario::CompletedRace => "completed",
                            TurnScenario::Interrupted => "inProgress",
                            _ => panic!("unexpected second turn query"),
                        }
                    } else if matches!(scenario, TurnScenario::Idle) {
                        "completed"
                    } else {
                        "inProgress"
                    };
                    queried = true;
                    json!({"data": [{"id": TURN, "status": status}]})
                }
                "turn/interrupt" => {
                    assert!(queried && !interrupted);
                    assert!(!matches!(
                        scenario,
                        TurnScenario::Idle | TurnScenario::FreshIdle
                    ));
                    assert_eq!(
                        request["params"],
                        json!({"threadId": THREAD, "turnId": TURN})
                    );
                    interrupted = true;
                    if matches!(scenario, TurnScenario::CompletedRace) {
                        response = json!({"id":id,"error":{"code":-32600,"message":"no active turn to interrupt"}});
                        socket
                            .send(ClientMessage::Text(response.to_string().into()))
                            .await
                            .unwrap();
                        continue;
                    }
                    if matches!(scenario, TurnScenario::Unsupported) {
                        response =
                            json!({"id":id,"error":{"code":-32601,"message":"Method not found"}});
                        socket
                            .send(ClientMessage::Text(response.to_string().into()))
                            .await
                            .unwrap();
                        continue;
                    }
                    if matches!(scenario, TurnScenario::Failed) {
                        socket
                            .send(completion(THREAD, TURN, "failed"))
                            .await
                            .unwrap();
                    }
                    json!({})
                }
                method => panic!("unexpected fake Codex request: {method}"),
            };
            if socket
                .send(ClientMessage::Text(response.to_string().into()))
                .await
                .is_err()
            {
                break;
            }
            if matches!(scenario, TurnScenario::Interrupted)
                && interrupted
                && request["method"] == "thread/turns/list"
            {
                // An empty acknowledgment and even a stale readback precede
                // completion. Ignore both wrong-thread and wrong-turn events.
                socket
                    .send(completion("other-thread", TURN, "completed"))
                    .await
                    .unwrap();
                socket
                    .send(completion(THREAD, "other-turn", "completed"))
                    .await
                    .unwrap();
                socket
                    .send(completion(THREAD, TURN, "interrupted"))
                    .await
                    .unwrap();
            }
        }
        (queried, interrupted)
    });
    let (handle, commands, ready) = codex_app_server::starting_control_channel();
    let control = tokio::spawn(codex_app_server::run_control(
        state.context.clone(),
        record,
        commands,
        ready,
    ));
    handle.wait_ready(Duration::from_secs(5)).await.unwrap();
    state.codex_controls.lock().unwrap().insert(
        "interrupt".into(),
        CodexControlEntry {
            launch_id: launch.clone(),
            handle,
        },
    );
    let body = json!({"expected_session_incarnation": launch});
    let app = router(state.clone());
    let (status, _) = call(app.clone(), post_json(URI, None, body.clone())).await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
    let (status, _) = call(
        app.clone(),
        post_json(
            URI,
            Some(TOKEN),
            json!({"expected_session_incarnation": "old-launch"}),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::CONFLICT);
    // The proxy holds this gate while a TUI turn/start is in flight.
    let gate = codex_app_server::acquire_account_mutation_gate(&state.context, &id).unwrap();
    let (status, refusal) = call(app.clone(), post_json(URI, Some(TOKEN), body.clone())).await;
    assert_eq!(status, StatusCode::CONFLICT);
    assert_eq!(refusal["error"]["code"], "codex-interrupt-busy");
    drop(gate);

    let (status, data) = call(app, post_json(URI, Some(TOKEN), body)).await;
    if let Some(outcome) = scenario.outcome() {
        assert_eq!(status, StatusCode::OK);
        assert_eq!(data["data"]["outcome"], outcome);
    } else {
        assert_eq!(status, StatusCode::CONFLICT);
        assert_eq!(data["error"]["code"], "codex-interrupt-unsupported");
        assert!(
            data["error"]["message"]
                .as_str()
                .unwrap()
                .contains("0.151.0")
        );
    }
    state.codex_controls.lock().unwrap().clear();
    control.await.unwrap().unwrap();
    assert_eq!(
        peer.await.unwrap(),
        (
            true,
            !matches!(scenario, TurnScenario::Idle | TurnScenario::FreshIdle)
        )
    );
}

fn completion(thread: &str, turn: &str, status: &str) -> ClientMessage {
    ClientMessage::Text(
        json!({"method":"turn/completed","params":{
            "threadId":thread,"turn":{"id":turn,"status":status}
        }})
        .to_string()
        .into(),
    )
}

#[test]
fn codex_interrupt_reports_the_exact_provider_terminal_status() {
    interrupt_round_trip(TurnScenario::Interrupted);
    interrupt_round_trip(TurnScenario::Failed);
}

#[test]
fn codex_interrupt_reports_completed_when_the_turn_wins_the_race() {
    interrupt_round_trip(TurnScenario::CompletedRace);
}

#[test]
fn codex_interrupt_is_an_explicit_no_op_on_an_idle_thread() {
    interrupt_round_trip(TurnScenario::Idle);
    interrupt_round_trip(TurnScenario::FreshIdle);
}

#[test]
fn codex_interrupt_refuses_an_unsupported_provider_method() {
    interrupt_round_trip(TurnScenario::Unsupported);
}

#[test]
fn completed_shadow_sidecars_refresh_activity_but_temporary_writes_do_not() {
    let shadow_replace = NotifyEvent::new(EventKind::Modify(notify::event::ModifyKind::Name(
        notify::event::RenameMode::Both,
    )))
    .add_path(PathBuf::from(
        "/state/sessions/example/.activity.shadow.json.tmp-1-2-0",
    ))
    .add_path(PathBuf::from(
        "/state/sessions/example/activity.shadow.json",
    ));
    assert!(activity_notify_event_relevant(&shadow_replace));
    let temporary = NotifyEvent::new(EventKind::Modify(notify::event::ModifyKind::Data(
        notify::event::DataChange::Any,
    )))
    .add_path(PathBuf::from(
        "/state/sessions/example/.activity.shadow.json.tmp-1-2-0",
    ));
    assert!(!activity_notify_event_relevant(&temporary));

    let session_replace = NotifyEvent::new(EventKind::Modify(notify::event::ModifyKind::Name(
        notify::event::RenameMode::Both,
    )))
    .add_path(PathBuf::from(
        "/state/sessions/example/.session.json.tmp-1-2-0",
    ))
    .add_path(PathBuf::from("/state/sessions/example/session.json"));
    assert!(activity_notify_event_relevant(&session_replace));
}

#[test]
fn activity_stream_shadow_resampling_does_not_broadcast_unchanged_state() {
    let mut turn = test_stream_turn_state(7, "2026-10-07T00:00:00Z");
    turn.source.provider = Some("claude".into());
    turn.shadow_observation = Some(crate::activity::StreamShadowObservationView {
        observer_version: "terminal-shadow.v1".into(),
        rule_id: "claude_working_indicator".into(),
        observed_at: "2026-10-07T00:00:00Z".into(),
        projection: "working".into(),
        disagrees: false,
    });
    let session = ActivityStreamSession {
        id: "shadow-session".into(),
        turn_state: Some(turn),
    };
    let log = ActivityEventLog::new(MACHINE.into(), vec![session.clone()]);
    let initial_sequence = log.sequence.load(Ordering::SeqCst);
    let mut resampled = session;
    resampled
        .turn_state
        .as_mut()
        .unwrap()
        .shadow_observation
        .as_mut()
        .unwrap()
        .observed_at = "2026-10-07T00:00:15Z".into();
    log.publish_snapshot(vec![resampled.clone()]);
    assert_eq!(
        log.sequence.load(Ordering::SeqCst),
        initial_sequence,
        "resampling an identical rule broadcast a full snapshot"
    );
    assert_eq!(
        log.state.lock().unwrap().latest_sessions,
        vec![resampled.clone()]
    );
    assert_eq!(
        log.state.lock().unwrap().latest_sessions[0]
            .turn_state
            .as_ref()
            .unwrap()
            .shadow_observation
            .as_ref()
            .unwrap()
            .observed_at,
        "2026-10-07T00:00:15Z"
    );
    let state = resampled.turn_state.as_mut().unwrap();
    state.phase = crate::activity::TurnPhase::Unknown;
    state.phase_changed_at = "2026-10-07T00:00:15Z".into();
    state.diagnostic = Some(crate::activity::StreamActivityDiagnosticView {
        completion_candidate_reason: None,
        reason: "claude_turn_outcome_unknown".into(),
        quiet: None,
    });
    log.publish_snapshot(vec![resampled.clone()]);
    assert_eq!(
        log.sequence.load(Ordering::SeqCst),
        initial_sequence + 1,
        "an actual phase change must reach stream subscribers"
    );
    resampled
        .turn_state
        .as_mut()
        .unwrap()
        .shadow_observation
        .as_mut()
        .unwrap()
        .rule_id = "claude_unmatched".into();
    log.publish_snapshot(vec![resampled]);
    assert_eq!(
        log.sequence.load(Ordering::SeqCst),
        initial_sequence + 2,
        "changed diagnostic evidence must reach stream subscribers"
    );
}

#[tokio::test(start_paused = true)]
async fn activity_refresh_timer_collects_without_hooks_or_http_polling() {
    let tmp = tempfile::TempDir::new().unwrap();
    let broker = ActivityBroker::for_test(MACHINE);
    let calls = Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let observed_calls = calls.clone();
    let collector: SessionCollector = Arc::new(move |_, _| {
        observed_calls.fetch_add(1, Ordering::SeqCst);
        Ok(Vec::new())
    });
    let (_tx, rx) = mpsc::channel(1);
    let (started_tx, mut started_rx) = mpsc::unbounded_channel();
    let task = tokio::spawn(activity_change_loop_inner(
        broker.log.clone(),
        broker.lifecycle.clone(),
        collector,
        CliContext {
            state_dir: tmp.path().to_path_buf(),
            host: None,
        },
        PathBuf::from("unused-tmux"),
        rx,
        ActivityChangeLoopControls {
            refresh_started: Some(started_tx),
            watch_rearm: None,
        },
    ));
    for _ in 0..4 {
        tokio::task::yield_now().await;
    }
    assert_eq!(calls.load(Ordering::SeqCst), 0);
    tokio::time::advance(Duration::from_millis(15_500)).await;
    let refreshed = tokio::time::timeout(Duration::from_secs(1), started_rx.recv()).await;
    task.abort();
    let _ = task.await;
    assert!(
        refreshed.is_ok(),
        "serve did not collect after the timer elapsed"
    );
}
