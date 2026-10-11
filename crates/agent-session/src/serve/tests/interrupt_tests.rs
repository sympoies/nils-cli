use super::*;

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
        reason: "interrupted_suspected".into(),
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
