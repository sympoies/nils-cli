// Exercise the real controller queue and native receiver over a local socket.
use super::*;
use futures_util::{SinkExt, StreamExt};
use pretty_assertions::assert_eq;

#[tokio::test]
async fn mailbox_native_start_and_steer_confirm_exact_rows_after_provider_ingress() {
    for working in [false, true] {
        let lock = GlobalStateLock::new();
        let _writers = EnvGuard::set(&lock, crate::notice::WRITERS_ENV, "1");
        let tmp = tempfile::tempdir_in("/tmp").unwrap();
        seed_session_with_runtime(tmp.path(), "alpha", "codex", "modeled-alpha");
        let launch = seed_codex_app_server_session(tmp.path(), "beta");
        let state = state(tmp.path(), Some(TOKEN), minimal_tmux(tmp.path()));
        let record = fixture::bind(&state.context, "beta", tmp.path(), "mailbox-thread");
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
        let listener = tokio::net::UnixListener::bind(tmp.path().join("beta.sock")).unwrap();
        let transcript = fixture::path(tmp.path(), "beta");
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
                        socket.send(ClientMessage::Text(json!({"method":"turn/started","params":{
                            "threadId":"mailbox-thread","turn":{"id":turn,"status":"inProgress"}
                        }}).to_string().into())).await.unwrap();
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
        let request = observed.try_recv().expect("one native mutation");
        assert_eq!(
            notification_fixture(tmp.path())["state"],
            "prompt_submitted"
        );
        let history = provider_history::stable_history_id("codex", None, "mailbox-thread");
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
            request["params"]["input"][0]["text"].as_str().unwrap()
        );
        assert!(!page.messages[0].human_prompt);
        drain_coordination_notifications(state).await;
        assert!(
            observed.try_recv().is_err(),
            "the consumed generation cannot write twice"
        );
        control.abort();
        let _ = control.await;
        peer.abort();
        let _ = peer.await;
    }
}
