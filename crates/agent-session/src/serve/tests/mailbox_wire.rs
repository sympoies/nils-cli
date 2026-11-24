// Exercise the real controller queue and native receiver over a local socket.
use super::*;
use crate::codex_app_server::tests::control_fixture;
use futures_util::{SinkExt, StreamExt};
use pretty_assertions::assert_eq;

#[test]
fn mailbox_native_start_and_steer_confirm_exact_rows_after_provider_ingress() {
    native_wakes(&[(false, None), (true, None)]);
}

#[test]
fn mailbox_native_draft_defers_until_composer_clears() {
    native_wakes(&[(false, Some(Draft::BeforeClaim))]);
}

#[test]
fn mailbox_native_draft_at_write_retries_without_starting_a_turn() {
    native_wakes(&[(false, Some(Draft::AfterObservation))]);
}

#[derive(Clone, Copy)]
enum Draft {
    BeforeClaim,
    AfterObservation,
}

fn native_wakes(cases: &[(bool, Option<Draft>)]) {
    for &(working, draft_boundary) in cases {
        control_fixture::run_short(async |root| exercise_native_wake(root, working, draft_boundary).await);
    }
}

async fn exercise_native_wake(root: &Path, working: bool, draft_boundary: Option<Draft>) {
        let lock = GlobalStateLock::new();
        let _writers = EnvGuard::set(&lock, crate::notice::WRITERS_ENV, "1");
        seed_session_with_runtime(root, "alpha", "codex", "modeled-alpha");
        let launch = seed_codex_app_server_session(root, "beta");
        let state = state(root, Some(TOKEN), fixture::composer_tmux(root, &launch));
        let composer = root.join("composer");
        let draft = "  Unsent operator text.  ".as_bytes();
        match draft_boundary {
            Some(Draft::BeforeClaim) => fs::write(&composer, draft).unwrap(),
            Some(Draft::AfterObservation) => {
                fs::write(root.join("draft-after-capture"), draft).unwrap()
            }
            None => {}
        }
        let record = fixture::bind(&state.context, "beta", root, "mailbox-thread");
        codex_app_server::replace_thread_binding(&record, "mailbox-thread").unwrap();
        activity::activate_runtime(&state.context, &record).unwrap();
        for kind in if working {
            vec!["turn_started"]
        } else {
            vec!["turn_started", "turn_completed"]
        } {
            activity::ingest_event(&state.context, "beta", serde_json::from_value(json!({
                "schema_version":activity::TURN_EVENT_VERSION,"event_id":format!("wire-{kind}"),"runtime_id":launch,
                "provider":"codex","provider_session_id":"mailbox-thread","provider_turn_id":"native-turn",
                "kind":kind,"confidence":"authoritative"
            })).unwrap()).unwrap();
        }
        let alpha = load_session_record(&state.context, "alpha").unwrap();
        let sender = provision_ready_coordination_fixture(&state.context, &alpha);
        provision_ready_coordination_fixture(&state.context, &record);
        let listener = tokio::net::UnixListener::bind(root.join("beta.sock")).unwrap();
        let transcript = fixture::path(root, "beta");
        let (sent, mut observed) = tokio::sync::mpsc::unbounded_channel();
        let peer = tokio::spawn(async move {
            let (stream, _) = listener.accept().await.unwrap();
            let mut socket = tokio_tungstenite::accept_async(stream).await.unwrap();
            while let Some(Ok(ClientMessage::Text(text))) = socket.next().await {
                let request: Value = serde_json::from_str(&text).unwrap();
                let Some(id) = request.get("id") else {
                    continue;
                };
                let result = match request["method"].as_str().unwrap() {
                    "initialize" | "thread/resume" => json!({}),
                    "thread/loaded/list" => json!({"data":["mailbox-thread"],"nextCursor":null}),
                    "account/rateLimits/read" => {
                        if working {
                            socket.send(ClientMessage::Text(json!({"method":"turn/started","params":{
                                "threadId":"mailbox-thread","turn":{"id":"native-turn","status":"inProgress"}
                            }}).to_string().into())).await.unwrap();
                        }
                        json!({"rateLimits":{}})
                    }
                    method @ ("turn/start" | "turn/steer") => {
                        assert_eq!(method, if working { "turn/steer" } else { "turn/start" });
                        let params = &request["params"];
                        let body = params["input"][0]["text"].as_str().unwrap();
                        let client = params["clientUserMessageId"]
                            .as_str()
                            .expect("both routes carry the attestation client id");
                        assert!(client.starts_with("agent-session-"));
                        let turn = if working { "native-turn" } else { "new-turn" };
                        if working {
                            assert_eq!(params["expectedTurnId"], "native-turn");
                        }
                        fixture::append(&transcript, body, client, turn);
                        // Ingress arrives before the acknowledgment. It must be able
                        // to acquire the record lock after the native write.
                        socket
                        .send(ClientMessage::Text(
                            json!({"method":"turn/started","params":{
                                "threadId":"mailbox-thread","turn":{"id":turn,"status":"inProgress"}
                            }})
                            .to_string()
                            .into(),
                        ))
                        .await
                        .unwrap();
                        sent.send(request.clone()).unwrap();
                        if working {
                            json!({"turnId":turn})
                        } else {
                            json!({"turn":{"id":turn}})
                        }
                    }
                    method => panic!("unexpected mailbox native method: {method}"),
                };
                if socket
                    .send(ClientMessage::Text(
                        json!({"id":id,"result":result}).to_string().into(),
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
            state.context.clone(),
            record,
            commands,
            ready,
        ));
        handle.wait_ready(Duration::from_secs(5)).await.unwrap();
        state.codex_controls.lock().unwrap().insert(
            "beta".into(),
            CodexControlEntry {
                launch_id: launch,
                handle,
            },
        );
        send_notification_fixture(&state, &sender, "native-mailbox-once").await;
        tokio::time::timeout(
            Duration::from_secs(20),
            drain_coordination_notifications(state.clone()),
        )
        .await
        .unwrap();
        if let Some(boundary) = draft_boundary {
            assert!(
                observed.try_recv().is_err(),
                "native turn started under an operator draft"
            );
            assert_eq!(fs::read(&composer).unwrap(), draft);
            let queued = notification_fixture(root);
            assert_eq!(queued["state"], "queued");
            assert_eq!(
                queued["attempted_generation"],
                match boundary {
                    Draft::BeforeClaim => json!(0),
                    Draft::AfterObservation => queued["generation"].clone(),
                },
            );
            assert_eq!(queued["last_reason"], "recipient-composer-not-empty");
            let attempt = queued["attempt_history"]
                .as_array()
                .unwrap()
                .last()
                .unwrap();
            assert_eq!(attempt["generation"], queued["generation"]);
            assert_eq!(attempt["reason"], "recipient-composer-not-empty");
            fs::write(&composer, "").unwrap();
            let due = std::time::UNIX_EPOCH
                + Duration::from_secs(queued["next_attempt_at_epoch"].as_u64().unwrap());
            tokio::time::sleep(
                due.duration_since(std::time::SystemTime::now())
                    .unwrap_or_default(),
            )
            .await;
            drain_coordination_notifications(state.clone()).await;
            assert_eq!(
                notification_fixture(root)["notified_generation"],
                queued["generation"]
            );
        }
        let request = observed.try_recv().expect("one native mutation");
        assert_eq!(
            notification_fixture(root)["state"],
            "prompt_submitted"
        );
        let history = provider_history::stable_history_id("codex", None, "mailbox-thread");
        let catalog = HistoryCatalog::new(
            vec![HistorySource {
                provider: "codex".into(),
                agent_profile: None,
                root: root.join("codex-home/sessions"),
            }],
            provider_history::archive_root(root),
            provider_history::star_root(root),
        )
        .with_session_state_dir(root.to_owned());
        let page = catalog
            .messages_with_parts(&history, None, 10, HistoryMessageDirection::Forward, true)
            .unwrap();
        assert_eq!(page.messages.len(), 1);
        assert_eq!(page.messages[0].role, "notice");
        assert_eq!(
            page.messages[0].text,
            request["params"]["input"][0]["text"].as_str().unwrap()
        );
        assert!(!page.messages[0].human_prompt);
        drain_coordination_notifications(state).await;
        assert!(
            observed.try_recv().is_err(),
            "the consumed generation cannot write twice"
        );
        assert!(!root.join("terminal-write").exists());
        control.abort();
        let _ = control.await;
        peer.abort();
        let _ = peer.await;
}
