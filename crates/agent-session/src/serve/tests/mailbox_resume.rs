use super::*;
use futures_util::{SinkExt, StreamExt};
use pretty_assertions::assert_eq;
use std::os::fd::AsRawFd;

struct ResumedFixture {
    temp: tempfile::TempDir,
    state: Arc<ServeState>,
    record: SessionRecord,
    commands: tokio::sync::mpsc::Receiver<codex_app_server::ControlCommand>,
    probe: tokio::task::JoinHandle<()>,
    _socket_dir: nils_test_support::tempdir::ScopedTempDir,
}

impl ResumedFixture {
    async fn new(provider_status: &'static str) -> Self {
        let temp = tempfile::TempDir::new().unwrap();
        seed_session_with_runtime(temp.path(), "alpha", "codex", "modeled-alpha");
        seed_codex_app_server_session(temp.path(), "beta");
        let state = state(temp.path(), Some(TOKEN), minimal_tmux(temp.path()));
        let mut record = load_session_record(&state.context, "beta").unwrap();
        crate::activity::activate_runtime(&state.context, &record).unwrap();
        for kind in ["turn_started", "turn_completed"] {
            let event = serde_json::from_value(json!({
                "schema_version": crate::activity::TURN_EVENT_VERSION,
                "event_id": format!("prior-{kind}"),
                "runtime_id": record.runtime.as_ref().unwrap().launch_id,
                "provider": "codex", "provider_session_id": "resumed-thread",
                "provider_turn_id": "prior-turn", "kind": kind,
                "confidence": "authoritative"
            })).unwrap();
            crate::activity::ingest_event(&state.context, &record.id, event).unwrap();
        }
        let (socket_dir, socket) = nils_test_support::short_socket_path("resume.sock");
        let runtime = record.runtime.as_mut().unwrap();
        runtime.launch_id = "resumed-runtime".into();
        runtime.generation += 1;
        runtime.extra.insert(codex_app_server::SOCKET_KEY.into(), json!(socket));
        record.provider_resume = Some(crate::ProviderResume {
            provider: "codex".into(), session_id: "resumed-thread".into(),
            captured_at: "2000-01-01T00:00:00Z".into(), capture_method: "test".into(),
            resume_args: vec![], extra: BTreeMap::new(),
        });
        crate::write_session_record(&state.context, &record).unwrap();
        record = fixture::bind(&state.context, "beta", temp.path(), "resumed-thread");
        let starting = crate::activity::activate_runtime(&state.context, &record).unwrap();
        assert_eq!(starting.phase, activity::TurnPhase::Starting);
        assert!(starting.current_turn.is_none());
        assert_eq!(starting.last_turn.as_ref().unwrap().outcome, "completed");
        let alpha = load_session_record(&state.context, "alpha").unwrap();
        let capability = provision_ready_coordination_fixture(&state.context, &alpha);
        provision_ready_coordination_fixture(&state.context, &record);
        send_notification_fixture(&state, &capability, "resumed-no-turn-end").await;
        let (handle, commands) = codex_app_server::control_channel();
        state.codex_controls.lock().unwrap().insert(record.id.clone(), CodexControlEntry {
            launch_id: record.runtime.as_ref().unwrap().launch_id.clone(), handle,
        });
        let listener = tokio::net::UnixListener::bind(socket).unwrap();
        let ingress_path = session_dir(&state.context, &record.id).join(".activity-ingress.lock");
        let probe = tokio::spawn(async move {
            let mut pending_ingress = None;
            loop {
                let (stream, _) = listener.accept().await.unwrap();
                let mut socket = tokio_tungstenite::accept_async(stream).await.unwrap();
                while let Some(Ok(message)) = socket.next().await {
                    let request: Value = serde_json::from_str(message.to_text().unwrap()).unwrap();
                    let method = request["method"].as_str().unwrap();
                    if method == "initialized" { continue; }
                    let result = match method {
                        "initialize" => json!({}),
                        "thread/loaded/list" => json!({"data": if provider_status == "loading" {
                            Vec::<&str>::new()
                        } else { vec!["resumed-thread"] }}),
                        "thread/read" => {
                            if provider_status == "pending" && pending_ingress.is_none() {
                                // A lifecycle writer advertises ingress before waiting
                                // behind the record lock held by notification admission.
                                let file = OpenOptions::new().create(true).truncate(false)
                                    .read(true).write(true).open(&ingress_path).unwrap();
                                // SAFETY: file owns the descriptor for this fixture lock.
                                assert_eq!(unsafe { libc::flock(file.as_raw_fd(), libc::LOCK_SH) }, 0);
                                pending_ingress = Some(file);
                            }
                            json!({"thread": {"id": "resumed-thread", "status": {"type":
                                if provider_status == "pending" { "idle" } else { provider_status }
                            }}})
                        }
                        other => panic!("idle observation must not send input: {other}"),
                    };
                    socket.send(tokio_tungstenite::tungstenite::Message::Text(
                        json!({"id": request["id"], "result": result}).to_string().into()
                    )).await.unwrap();
                }
            }
        });
        Self { temp, state, record, commands, probe, _socket_dir: socket_dir }
    }

    fn retry_now(&self) {
        let path = self.temp.path().join("coordination/registry.json");
        let mut registry: Value = serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
        for notification in registry["notifications"].as_object_mut().unwrap().values_mut() {
            notification["next_attempt_at_epoch"] = json!(0);
        }
        fs::write(path, serde_json::to_vec(&registry).unwrap()).unwrap();
    }
}

impl Drop for ResumedFixture {
    fn drop(&mut self) { self.probe.abort(); }
}

#[tokio::test]
async fn resumed_codex_without_turn_end_delivers_one_wake_after_observed_idle() {
    let lock = GlobalStateLock::new();
    let _writers = EnvGuard::set(&lock, crate::notice::WRITERS_ENV, "1");
    let mut fixture = ResumedFixture::new("idle").await;
    let state = fixture.state.clone();
    let drain = tokio::spawn(async move { drain_coordination_notifications(state).await });
    tokio::select! {
        biased;
        result = fixture.commands.recv() => {
            let Some(codex_app_server::ControlCommand::Prompt { message, client_message_id, response, .. }) = result else {
                panic!("expected one wake");
            };
            assert!(crate::notice::parse_notice(&message).is_some());
            assert!(!message.contains("notification reconciliation fixture"));
            fixture::append(&fixture::path(fixture.temp.path(), "beta"), &message, client_message_id.as_deref().unwrap(), "wake-turn");
            response.send(Ok("wake-turn".into())).unwrap();
        }
        () = async { while !drain.is_finished() { tokio::task::yield_now().await; } } => {}
    }
    drain.await.unwrap();
    let notification = notification_fixture(fixture.temp.path());
    assert_eq!(notification["state"], "prompt_submitted", "{notification}");
    assert_eq!(notification["notified_generation"], 1);
    let idle = activity::state_for_view(&fixture.state.context, &fixture.record).unwrap();
    assert_eq!(idle.phase, activity::TurnPhase::Waiting);
    assert!(idle.current_turn.is_none());
    assert_eq!(idle.last_turn.as_ref().unwrap().outcome, "completed");
    fixture.retry_now();
    drain_coordination_notifications(fixture.state.clone()).await;
    assert!(fixture.commands.try_recv().is_err(), "wake must not repeat");
}

#[tokio::test]
async fn resumed_codex_busy_or_loading_without_turn_end_defers_wake() {
    let lock = GlobalStateLock::new();
    let _writers = EnvGuard::set(&lock, crate::notice::WRITERS_ENV, "1");
    for status in ["active", "loading", "pending"] {
        let mut fixture = ResumedFixture::new(status).await;
        for _ in 0..2 {
            fixture.retry_now();
            drain_coordination_notifications(fixture.state.clone()).await;
            assert_eq!(notification_fixture(fixture.temp.path())["notified_generation"], 0);
            assert!(fixture.commands.try_recv().is_err());
            // This generation's fixture start predates the observation window.
            let published = activity::state_for_view(&fixture.state.context, &fixture.record).unwrap();
            assert_eq!(published.phase, activity::TurnPhase::Unknown);
            assert_eq!(published.diagnostic.as_ref().unwrap().reason, "codex_startup_idle_unverified");
            assert_eq!(activity::controller_state_for_record(&fixture.state.context, &fixture.record).unwrap().phase,
                activity::TurnPhase::Starting);
        }
    }
}
