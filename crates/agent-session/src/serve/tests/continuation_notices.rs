//! Scheduler producer boundaries; synthetic receivers only.
use super::prompt_receipts_tests::guard_tmux;
use super::*;
use pretty_assertions::assert_eq;
#[path = "continuation_claude.rs"]
mod claude;

#[tokio::test]
async fn codex_capacity_scheduler_submits_the_fixed_prompt_without_usage_lookup() {
    let lock = GlobalStateLock::new();
    let _writers = EnvGuard::set(&lock, crate::notice::WRITERS_ENV, "1");
    let tmp = tempfile::TempDir::new().unwrap();
    let _home = EnvGuard::set(
        &lock,
        "CODEX_HOME",
        tmp.path().join("codex-home").to_str().unwrap(),
    );
    let launch_id = seed_codex_app_server_session(tmp.path(), "codex-capacity-recovery");
    let st = state(
        tmp.path(),
        Some(TOKEN),
        codex_tmux(tmp.path(), "codex-capacity-recovery"),
    );
    let record = bind_codex(&st.context, "codex-capacity-recovery", tmp.path());
    crate::activity::activate_runtime(&st.context, &record).unwrap();
    let failure = crate::activity::ingest_codex_app_server_failure_with_kind(
        &st.context,
        &record.id,
        &launch_id,
        "thread-capacity",
        "turn-capacity",
        codex_app_server::StructuredFailureKind::ProviderCapacity,
    )
    .unwrap();
    let blocked_turn_id = failure
        .turn_state
        .last_turn
        .and_then(|turn| turn.provider_turn_id)
        .unwrap();
    auto_resume::set_enabled(&st.context, &record.id, true, "2020-01-01T00:00:00Z").unwrap();
    let revision = crate::activity::state_for_view(&st.context, &record)
        .unwrap()
        .revision;
    auto_resume::arm_provider_capacity(
        &st.context,
        &record.id,
        &launch_id,
        &crate::codex_account::binding_snapshot(&record),
        blocked_turn_id,
        revision,
        "2020-01-01T00:00:01Z",
    )
    .unwrap();

    let (handle, mut commands) = codex_app_server::control_channel();
    st.codex_controls.lock().unwrap().insert(
        record.id.clone(),
        CodexControlEntry {
            launch_id: launch_id.clone(),
            handle,
        },
    );
    let transcript = transcript(tmp.path());
    let responder = tokio::spawn(async move {
        let Some(codex_app_server::ControlCommand::NoticeContinue {
            message,
            client_message_id,
            response,
            ..
        }) = commands.recv().await
        else {
            panic!("capacity recovery must submit directly without requesting usage");
        };
        let parsed = crate::notice::parse_notice(&message)
            .expect("capacity producer must submit a canonical notice");
        assert_eq!(parsed.metadata.kind(), "runtime.resume");
        assert_eq!(parsed.body, auto_resume::CAPACITY_CONTINUATION_MESSAGE);
        append_notice(&transcript, &message, &client_message_id);
        response.send(Ok("continued-turn".to_string())).unwrap();
    });

    process_codex_capacity_auto_resume_id(
        st.clone(),
        CodexAutoResumeTarget {
            id: record.id.clone(),
            launch_id,
            binding: crate::codex_account::binding_snapshot(&record),
        },
    )
    .await;
    responder.await.unwrap();

    assert_eq!(
        auto_resume::view_for_record(&st.context, &record).state,
        "resumed"
    );
}

#[test]
fn generic_auto_resume_refuses_unsupported_notice_transport_without_raw_input() {
    let lock = GlobalStateLock::new();
    let _writers = EnvGuard::set(&lock, crate::notice::WRITERS_ENV, "1");
    let tmp = tempfile::tempdir().unwrap();
    seed_claude_prompt_session(tmp.path(), "generic-notice");
    let tmux = guard_tmux(tmp.path(), "generic-notice");
    let st = state(tmp.path(), Some(TOKEN), tmux.clone());
    let record = load_session_record(&st.context, "generic-notice").unwrap();
    let result = crate::send_auto_resume_input(
        &st.context,
        &record,
        crate::acquire_session_record_lock(&st.context, &record.id).unwrap(),
        auto_resume::CONTINUATION_MESSAGE,
        &tmux,
    );
    let error = result.expect_err("unsupported notice transport must refuse instead of raw input");
    assert_eq!(error.code(), "notice-transport-unsupported");
    assert!(!tmp.path().join("submitted").exists());
}

#[tokio::test]
async fn codex_usage_scheduler_submits_a_canonical_notice() {
    let lock = GlobalStateLock::new();
    let _writers = EnvGuard::set(&lock, crate::notice::WRITERS_ENV, "1");
    let tmp = tempfile::TempDir::new().unwrap();
    let _home = EnvGuard::set(
        &lock,
        "CODEX_HOME",
        tmp.path().join("codex-home").to_str().unwrap(),
    );
    let launch_id = seed_codex_app_server_session(tmp.path(), "codex-usage-recovery");
    let st = state(
        tmp.path(),
        Some(TOKEN),
        codex_tmux(tmp.path(), "codex-usage-recovery"),
    );
    let record = bind_codex(&st.context, "codex-usage-recovery", tmp.path());
    crate::activity::activate_runtime(&st.context, &record).unwrap();
    let failure = crate::activity::ingest_codex_app_server_failure_with_kind(
        &st.context,
        &record.id,
        &launch_id,
        "thread-capacity",
        "turn-capacity",
        codex_app_server::StructuredFailureKind::UsageExhausted,
    )
    .unwrap();
    let blocked_turn_id = failure
        .turn_state
        .last_turn
        .and_then(|turn| turn.provider_turn_id)
        .unwrap();
    auto_resume::set_enabled(&st.context, &record.id, true, "2020-01-01T00:00:00Z").unwrap();
    let revision = crate::activity::state_for_view(&st.context, &record)
        .unwrap()
        .revision;
    auto_resume::arm_usage_exhaustion(
        &st.context,
        &record.id,
        blocked_turn_id,
        revision,
        "2020-01-01T00:00:01Z",
    )
    .unwrap();
    auto_resume::tick_for_runtime_and_binding(
        &st.context,
        &record.id,
        &launch_id,
        auto_resume::RuntimeBindingTick {
            binding: &crate::codex_account::binding_snapshot(&record),
            failover_account: None,
        },
        1_577_836_801,
        &UsageSnapshot {
            authoritative: true,
            has_exhausted_windows: true,
            exhausted_reset_epochs: vec![1_577_836_810],
            soonest_reset_epoch: None,
        },
        |_, _lock| panic!("initial usage tick must schedule, not submit"),
    )
    .unwrap();

    let (handle, mut commands) = codex_app_server::control_channel();
    st.codex_controls.lock().unwrap().insert(
        record.id.clone(),
        CodexControlEntry {
            launch_id: launch_id.clone(),
            handle,
        },
    );
    let transcript = transcript(tmp.path());
    let responder = tokio::spawn(async move {
        let Some(codex_app_server::ControlCommand::Usage(response)) = commands.recv().await else {
            panic!("usage recovery must refresh usage before notice delivery");
        };
        response
            .send(Ok(UsageSnapshot {
                authoritative: true,
                has_exhausted_windows: false,
                exhausted_reset_epochs: Vec::new(),
                soonest_reset_epoch: None,
            }))
            .unwrap();
        let Some(codex_app_server::ControlCommand::NoticeContinue {
            message,
            client_message_id,
            response,
            ..
        }) = commands.recv().await
        else {
            panic!("capacity recovery must submit directly without requesting usage");
        };
        let parsed = crate::notice::parse_notice(&message)
            .expect("usage producer must submit a canonical notice");
        assert_eq!(parsed.metadata.kind(), "runtime.resume");
        assert_eq!(parsed.body, auto_resume::CONTINUATION_MESSAGE);
        append_notice(&transcript, &message, &client_message_id);
        response.send(Ok("continued-turn".to_string())).unwrap();
    });

    process_codex_auto_resume_id(
        st.clone(),
        CodexAutoResumeTarget {
            id: record.id.clone(),
            launch_id,
            binding: crate::codex_account::binding_snapshot(&record),
        },
    )
    .await;
    responder.await.unwrap();

    assert_eq!(
        auto_resume::view_for_record(&st.context, &record).state,
        "resumed"
    );
}

fn transcript(root: &Path) -> PathBuf {
    root.join("codex-home/sessions/2026/01/rollout.jsonl")
}

fn bind_codex(context: &CliContext, id: &str, root: &Path) -> SessionRecord {
    let mut record = load_session_record(context, id).unwrap();
    record.provider_resume = Some(
        serde_json::from_value(json!({"provider":"codex", "session_id":"thread-capacity",
        "captured_at":"2020-01-01T00:00:00Z","capture_method":"fixture","resume_args":[]}))
        .unwrap(),
    );
    crate::write_session_record(context, &record).unwrap();
    let path = transcript(root);
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    fs::write(path, format!("{}\n", json!({"type":"session_meta","timestamp":"2026-01-01T00:00:00Z",
        "payload":{"id":"thread-capacity","cwd":record.cwd,"source":"cli","timestamp":"2026-01-01T00:00:00Z"}}))).unwrap();
    record
}

fn append_notice(path: &Path, text: &str, client: &str) {
    let mut file = fs::OpenOptions::new().append(true).open(path).unwrap();
    writeln!(file,"{}",json!({"type":"response_item","payload":{"type":"message","role":"user",
        "content":[{"type":"input_text","text":text}],
        "internal_chat_message_metadata_passthrough":{"turn_id":"continued-turn","content_item_kinds":["user.text"]}}})).unwrap();
    writeln!(file,"{}",json!({"type":"event_msg","payload":{"type":"user_message","client_id":client,"message":text}})).unwrap();
}

fn codex_tmux(root: &Path, id: &str) -> PathBuf {
    let tmux = guard_tmux(root, id);
    let script = fs::read_to_string(&tmux).unwrap().replace(
        "print('────────────────────────────\\n❯ ' + text + '\\n────────────────────────────')",
        "print('› ' + text + '\\n\\n  status')",
    );
    fs::write(&tmux, script).unwrap();
    tmux
}

#[tokio::test]
async fn capacity_notice_native_receiver_admits_only_its_interrupted_non_idle_turn() {
    for related in [true, false] {
        let lock = GlobalStateLock::new();
        let _writers = EnvGuard::set(&lock, crate::notice::WRITERS_ENV, "1");
        let tmp = tempfile::tempdir_in("/tmp").unwrap();
        let _home = EnvGuard::set(
            &lock,
            "CODEX_HOME",
            tmp.path().join("codex-home").to_str().unwrap(),
        );
        let id = "capacity-wire";
        let launch = seed_codex_app_server_session(tmp.path(), id);
        let st = state(tmp.path(), Some(TOKEN), codex_tmux(tmp.path(), id));
        let record = bind_codex(&st.context, id, tmp.path());
        activity::activate_runtime(&st.context, &record).unwrap();
        let listener =
            tokio::net::UnixListener::bind(tmp.path().join(format!("{id}.sock"))).unwrap();
        let path = transcript(tmp.path());
        let (sent, mut received) = tokio::sync::mpsc::unbounded_channel();
        let peer = tokio::spawn(async move {
            let (stream, _) = listener.accept().await.unwrap();
            let mut socket = tokio_tungstenite::accept_async(stream).await.unwrap();
            while let Some(Ok(ClientMessage::Text(text))) = socket.next().await {
                let request: Value = serde_json::from_str(&text).unwrap();
                let Some(request_id) = request.get("id") else {
                    continue;
                };
                let result = match request["method"].as_str().unwrap() {
                    "initialize" | "thread/resume" => json!({}),
                    "thread/loaded/list" => json!({"data":["thread-capacity"],"nextCursor":null}),
                    "account/rateLimits/read" => {
                        for notification in [
                            json!({"method":"turn/started","params":{"threadId":"thread-capacity","turn":{"id":if related {"turn-capacity"} else {"unrelated-turn"},"status":"inProgress"}}}),
                            json!({"method":"error","params":{"threadId":"thread-capacity","turnId":if related {"turn-capacity"} else {"unrelated-turn"},
                                "willRetry":false,"error":{"codexErrorInfo":"serverOverloaded"}}}),
                        ] {
                            socket
                                .send(ClientMessage::Text(notification.to_string().into()))
                                .await
                                .unwrap();
                        }
                        json!({"rateLimits":{}})
                    }
                    "turn/start" => {
                        let params = &request["params"];
                        let text = params["input"][0]["text"].as_str().unwrap();
                        let notice = crate::notice::parse_notice(text)
                            .expect("native receiver must get an envelope");
                        assert_eq!(notice.body, auto_resume::CAPACITY_CONTINUATION_MESSAGE);
                        append_notice(
                            &path,
                            text,
                            params["clientUserMessageId"]
                                .as_str()
                                .expect("notice client id"),
                        );
                        sent.send(request.clone()).unwrap();
                        json!({"turn":{"id":"continued-turn"}})
                    }
                    method => panic!("unexpected scheduler RPC: {method}"),
                };
                if socket
                    .send(ClientMessage::Text(
                        json!({"id":request_id,"result":result}).to_string().into(),
                    ))
                    .await
                    .is_err()
                {
                    break;
                }
            }
        });
        let (handle, commands, ready) = codex_app_server::starting_control_channel();
        let control = tokio::spawn(codex_app_server::run_control(
            st.context.clone(),
            record.clone(),
            commands,
            ready,
        ));
        handle.wait_ready(Duration::from_secs(5)).await.unwrap();
        st.codex_controls.lock().unwrap().insert(
            id.into(),
            CodexControlEntry {
                launch_id: launch.clone(),
                handle,
            },
        );
        let failure = activity::ingest_codex_app_server_failure_with_kind(
            &st.context,
            id,
            &launch,
            "thread-capacity",
            "turn-capacity",
            codex_app_server::StructuredFailureKind::ProviderCapacity,
        )
        .unwrap();
        let predecessor = failure
            .turn_state
            .last_turn
            .unwrap()
            .provider_turn_id
            .unwrap();
        let revision = activity::state_for_view(&st.context, &record)
            .unwrap()
            .revision;
        auto_resume::set_enabled(&st.context, id, true, "2020-01-01T00:00:00Z").unwrap();
        auto_resume::arm_provider_capacity(
            &st.context,
            id,
            &launch,
            &crate::codex_account::binding_snapshot(&record),
            predecessor,
            revision,
            "2020-01-01T00:00:01Z",
        )
        .unwrap();
        let target = || CodexAutoResumeTarget {
            id: id.into(),
            launch_id: launch.clone(),
            binding: crate::codex_account::binding_snapshot(&record),
        };
        process_codex_capacity_auto_resume_id(st.clone(), target()).await;
        let view = auto_resume::view_for_record(&st.context, &record);
        assert_eq!(
            view.state,
            if related {
                "resumed"
            } else {
                "terminal_failure"
            }
        );
        if related {
            let sent = received.try_recv().unwrap();
            let history = provider_history::stable_history_id("codex", None, "thread-capacity");
            let catalog = HistoryCatalog::new(
                vec![HistorySource {
                    provider: "codex".into(),
                    agent_profile: None,
                    root: tmp.path().join("codex-home/sessions"),
                }],
                provider_history::archive_root(tmp.path()),
                provider_history::star_root(tmp.path()),
            )
            .with_session_state_dir(tmp.path().to_owned());
            let page = catalog
                .messages_with_parts(&history, None, 10, HistoryMessageDirection::Forward, true)
                .unwrap();
            assert_eq!(page.messages.len(), 1);
            assert_eq!(page.messages[0].role, "notice");
            assert_eq!(
                page.messages[0].text,
                sent["params"]["input"][0]["text"].as_str().unwrap()
            );
            assert!(!page.messages[0].human_prompt);
        }
        assert!(received.try_recv().is_err());
        process_codex_capacity_auto_resume_id(st.clone(), target()).await;
        assert!(
            received.try_recv().is_err(),
            "the generation cannot dispatch twice"
        );
        control.abort();
        let _ = control.await;
        peer.abort();
        let _ = peer.await;
    }
}

#[tokio::test]
async fn capacity_notice_does_not_repair_an_observed_stale_draft() {
    let lock = GlobalStateLock::new();
    let _writers = EnvGuard::set(&lock, crate::notice::WRITERS_ENV, "1");
    let tmp = tempfile::TempDir::new().unwrap();
    let _home = EnvGuard::set(
        &lock,
        "CODEX_HOME",
        tmp.path().join("codex-home").to_str().unwrap(),
    );
    let launch_id = seed_codex_app_server_session(tmp.path(), "codex-capacity-recovery");
    let st = state(
        tmp.path(),
        Some(TOKEN),
        codex_tmux(tmp.path(), "codex-capacity-recovery"),
    );
    let record = bind_codex(&st.context, "codex-capacity-recovery", tmp.path());
    crate::activity::activate_runtime(&st.context, &record).unwrap();
    let failure = crate::activity::ingest_codex_app_server_failure_with_kind(
        &st.context,
        &record.id,
        &launch_id,
        "thread-capacity",
        "turn-capacity",
        codex_app_server::StructuredFailureKind::ProviderCapacity,
    )
    .unwrap();
    let blocked_turn_id = failure
        .turn_state
        .last_turn
        .and_then(|turn| turn.provider_turn_id)
        .unwrap();
    auto_resume::set_enabled(&st.context, &record.id, true, "2020-01-01T00:00:00Z").unwrap();
    let revision = crate::activity::state_for_view(&st.context, &record)
        .unwrap()
        .revision;
    auto_resume::arm_provider_capacity(
        &st.context,
        &record.id,
        &launch_id,
        &crate::codex_account::binding_snapshot(&record),
        blocked_turn_id,
        revision,
        "2020-01-01T00:00:01Z",
    )
    .unwrap();

    let draft = "untouched owner draft";
    fs::write(tmp.path().join("composer"), draft).unwrap();
    let _bound = EnvGuard::set(&lock, crate::send_guard::drafts::BOUND_ENV, "1");
    crate::send_guard::drafts::observe_at(&st.context, &record, Some(draft), 1, 0).unwrap();
    let (handle, mut commands) = codex_app_server::control_channel();
    st.codex_controls.lock().unwrap().insert(
        record.id.clone(),
        CodexControlEntry {
            launch_id: launch_id.clone(),
            handle,
        },
    );
    let writes = Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let observed = writes.clone();
    let peer = tokio::spawn(async move {
        if let Some(codex_app_server::ControlCommand::NoticeContinue { response, .. }) =
            commands.recv().await
        {
            observed.fetch_add(1, Ordering::SeqCst);
            let _ = response.send(Err("fixture input must be refused".into()));
        }
    });
    process_codex_capacity_auto_resume_id(
        st.clone(),
        CodexAutoResumeTarget {
            id: record.id.clone(),
            launch_id,
            binding: crate::codex_account::binding_snapshot(&record),
        },
    )
    .await;
    peer.abort();
    let _ = peer.await;
    assert!(
        crate::send_guard::drafts::list(&st.context, &record)
            .unwrap()
            .is_empty(),
        "scheduler notice must not stash or repair an occupied stale draft"
    );
    assert_eq!(
        fs::read_to_string(tmp.path().join("composer")).unwrap(),
        draft
    );
    let calls = fs::read_to_string(tmp.path().join("transport-calls")).unwrap();
    assert!(
        !calls
            .lines()
            .any(|line| serde_json::from_str::<Value>(line).unwrap()[0] == "send-keys")
    );
    assert_eq!(writes.load(Ordering::SeqCst), 0);
}
