//! Synthetic HTTP/protocol coverage: no live provider or acceptance claim.
use super::*;
use pretty_assertions::{assert_eq, assert_ne};

const ID: &str = "native-busy-reply";
const THREAD: &str = "native-busy-thread";
const TURN: &str = "native-busy-turn";
const TEXT: &str = "inspect the next failing test";

fn busy_event(launch: &str, provider: &str) -> activity::TurnEvent {
    serde_json::from_value(json!({
        "schema_version": activity::TURN_EVENT_VERSION,
        "event_id": "native-busy-start",
        "runtime_id": launch,
        "provider": provider,
        "provider_turn_id": TURN,
        "kind": "turn_started",
        "confidence": "authoritative"
    }))
    .unwrap()
}

fn body(launch: &str, request: &str) -> Value {
    json!({"text": TEXT, "expected_session_incarnation": launch, "request_id": request})
}

fn assert_working(state: &ServeState) {
    let record = load_session_record(&state.context, ID).unwrap();
    let turn = activity::state_for_view(&state.context, &record).unwrap();
    assert_eq!(turn.phase, activity::TurnPhase::Working);
    assert!(turn.current_turn.is_some());
}

async fn replay_and_lookup(state: Arc<ServeState>, launch: &str, expected: &Value) {
    let uri = format!("/sessions/{ID}/prompt/v2");
    let (_, replay) = call(
        router(state.clone()),
        post_json(&uri, Some(TOKEN), body(launch, "busy-reply")),
    )
    .await;
    assert_eq!(&replay, expected);
    let (_, lookup) = call(
        router(state),
        bearer_get(&format!(
            "{uri}/receipts/busy-reply?expected_session_incarnation={launch}"
        )),
    )
    .await;
    let receipt = expected
        .pointer("/data/receipt")
        .or_else(|| expected.pointer("/error/details/receipt"))
        .unwrap();
    assert_eq!(&lookup["data"]["receipt"], receipt);
}

async fn read_json<S>(socket: &mut S) -> Value
where
    S: futures_util::Stream<Item = Result<ClientMessage, tokio_tungstenite::tungstenite::Error>>
        + Unpin,
{
    let message = socket.next().await.unwrap().unwrap();
    serde_json::from_str(message.to_text().unwrap()).unwrap()
}

async fn reply<S>(socket: &mut S, request: &Value, result: Value)
where
    S: futures_util::Sink<ClientMessage, Error = tokio_tungstenite::tungstenite::Error> + Unpin,
{
    socket
        .send(ClientMessage::Text(
            json!({"id": request["id"], "result": result})
                .to_string()
                .into(),
        ))
        .await
        .unwrap();
}

#[tokio::test]
async fn keyed_codex_reply_reaches_native_protocol_while_turn_is_working() {
    for acknowledgement in ["accepted", "unproven", "closed"] {
        let acknowledge = acknowledgement == "accepted";
        let tmp = tempfile::tempdir_in("/tmp").unwrap();
        let launch = seed_codex_app_server_session(tmp.path(), ID);
        let tmux = guard_tmux(tmp.path(), ID);
        let script = fs::read_to_string(&tmux).unwrap().replace(
            "print('────────────────────────────\\n❯ ' + text + '\\n────────────────────────────')",
            "print('› ' + text + '\\n\\n  status')",
        );
        fs::write(&tmux, script).unwrap();
        let st = state(tmp.path(), Some(TOKEN), tmux);
        let record = load_session_record(&st.context, ID).unwrap();
        activity::activate_runtime(&st.context, &record).unwrap();
        // Provider hooks and the native control stream are separate inputs.
        // Establish both the durable hook activity and the protocol's active turn.
        activity::ingest_event(&st.context, ID, busy_event(&launch, "codex")).unwrap();
        codex_app_server::replace_thread_binding(&record, THREAD).unwrap();
        let listener =
            tokio::net::UnixListener::bind(tmp.path().join(format!("{ID}.sock"))).unwrap();
        let (seen, mut received) = tokio::sync::mpsc::unbounded_channel();
        let peer =
            tokio::spawn(async move {
                let (stream, _) = listener.accept().await.unwrap();
                let mut socket = tokio_tungstenite::accept_async(stream).await.unwrap();
                let initialize = read_json(&mut socket).await;
                assert_eq!(initialize["method"], "initialize");
                reply(&mut socket, &initialize, json!({})).await;
                assert_eq!(read_json(&mut socket).await["method"], "initialized");
                let loaded = read_json(&mut socket).await;
                assert_eq!(loaded["method"], "thread/loaded/list");
                reply(
                    &mut socket,
                    &loaded,
                    json!({"data": [THREAD], "nextCursor": null}),
                )
                .await;
                let resume = read_json(&mut socket).await;
                assert_eq!(resume["method"], "thread/resume");
                assert_eq!(resume["params"]["threadId"], THREAD);
                reply(&mut socket, &resume, json!({})).await;
                let usage = read_json(&mut socket).await;
                assert_eq!(usage["method"], "account/rateLimits/read");
                socket.send(ClientMessage::Text(json!({
                "method": "turn/started",
                "params": {"threadId": THREAD, "turn": {"id": TURN, "status": "inProgress"}}
            }).to_string().into())).await.unwrap();
                reply(&mut socket, &usage, json!({"rateLimits": {}})).await;
                let prompt = read_json(&mut socket).await;
                assert_eq!(prompt["method"], "turn/start");
                assert_eq!(prompt["params"]["threadId"], THREAD);
                assert_eq!(prompt["params"]["input"][0]["text"], TEXT);
                // Interleaved reminder input and a sibling input with identical
                // text are notifications, not this request's native RPC response.
                for (item, text) in [
                    ("reminder-input", "Mailbox: agent-session message inbox"),
                    ("sibling-input", TEXT),
                ] {
                    socket
                        .send(ClientMessage::Text(
                            json!({
                                "method": "item/completed",
                                "params": {"threadId": THREAD, "turnId": TURN,
                                    "item": {"id": item, "type": "userMessage",
                                        "content": [{"type": "text", "text": text}]}}
                            })
                            .to_string()
                            .into(),
                        ))
                        .await
                        .unwrap();
                }
                seen.send(prompt).unwrap();
                (socket, seen)
            });
        let (handle, commands, ready) = codex_app_server::starting_control_channel();
        let control = tokio::spawn(codex_app_server::run_control(
            st.context.clone(),
            record,
            commands,
            ready,
        ));
        handle.wait_ready(Duration::from_secs(5)).await.unwrap();
        st.codex_controls.lock().unwrap().insert(
            ID.into(),
            CodexControlEntry {
                launch_id: launch.clone(),
                handle,
            },
        );
        assert_working(&st);
        let uri = format!("/sessions/{ID}/prompt/v2");
        let (status, refused) = call(
            router(st.clone()),
            post_json(&uri, Some(TOKEN), body("replaced-runtime", "stale-runtime")),
        )
        .await;
        assert_eq!(status, StatusCode::CONFLICT, "{refused}");
        assert_eq!(refused["error"]["code"], "session-incarnation-conflict");
        fs::write(tmp.path().join("composer"), "owner draft").unwrap();
        let (status, refused) = call(
            router(st.clone()),
            post_json(&uri, Some(TOKEN), body(&launch, "draft-refusal")),
        )
        .await;
        assert_eq!(status, StatusCode::CONFLICT, "{refused}");
        assert_eq!(refused["error"]["code"], "composer-not-empty");
        assert_eq!(
            fs::read_to_string(tmp.path().join("composer")).unwrap(),
            "owner draft"
        );
        assert!(received.try_recv().is_err());
        fs::write(tmp.path().join("composer"), "").unwrap();
        let request = tokio::spawn(call(
            router(st.clone()),
            post_json(
                &format!("/sessions/{ID}/prompt/v2"),
                Some(TOKEN),
                body(&launch, "busy-reply"),
            ),
        ));
        let native = tokio::time::timeout(Duration::from_secs(5), received.recv())
            .await
            .unwrap()
            .unwrap();
        assert_working(&st);
        let (mut socket, _seen) = peer.await.unwrap();
        if acknowledge {
            // Model native same-turn acceptance without a completed-turn
            // event or an interrupt. A lost ACK must remain unknown.
            reply(
                &mut socket,
                &native,
                json!({"turn": {"id": TURN, "status": "inProgress"}}),
            )
            .await;
        } else if acknowledgement == "unproven" {
            reply(
                &mut socket,
                &native,
                json!({"turn": {"status": "inProgress"}}),
            )
            .await;
        } else {
            socket.close(None).await.unwrap();
        }
        let (status, result) = request.await.unwrap();
        assert_eq!(
            status,
            if acknowledge {
                StatusCode::OK
            } else {
                StatusCode::INTERNAL_SERVER_ERROR
            },
            "{result}"
        );
        assert_working(&st);
        replay_and_lookup(st.clone(), &launch, &result).await;
        assert!(
            received.try_recv().is_err(),
            "a replay must not dispatch another request"
        );
        if acknowledgement != "closed" {
            assert!(
                tokio::time::timeout(Duration::from_millis(100), socket.next())
                    .await
                    .is_err(),
                "replay must not emit another native request"
            );
        }
        if acknowledgement == "unproven" {
            // A distinct authorized follow-up uses its own key. Its native ACK
            // cannot repair the earlier request's missing correlation.
            let later = tokio::spawn(call(
                router(st.clone()),
                post_json(&uri, Some(TOKEN), body(&launch, "later-reply")),
            ));
            let next = tokio::time::timeout(Duration::from_secs(5), read_json(&mut socket))
                .await
                .unwrap();
            assert_eq!(next["method"], "turn/start");
            assert_eq!(next["params"]["threadId"], THREAD);
            assert_ne!(next["id"], native["id"]);
            reply(
                &mut socket,
                &next,
                json!({"turn": {"id": TURN, "status": "inProgress"}}),
            )
            .await;
            let (status, accepted) = later.await.unwrap();
            assert_eq!(status, StatusCode::OK, "{accepted}");
            assert_eq!(accepted["data"]["receipt"]["request_id"], "later-reply");
            assert_eq!(accepted["data"]["receipt"]["outcome"], "submitted");
            replay_and_lookup(st.clone(), &launch, &result).await;
            assert_working(&st);
        }
        control.abort();
        let _ = control.await;
    }
}

#[tokio::test]
async fn keyed_claude_reply_uses_native_queue_without_waiting_for_idle() {
    let tmp = tempfile::TempDir::new().unwrap();
    let launch = seed_claude_prompt_session(tmp.path(), ID);
    let tmux = guard_tmux(tmp.path(), ID);
    let queue = format!(
        "  {TEXT}\n  ctrl+x ctrl+s to send now\n· Working… (running tool)\n\
         ────────────────────────────\n❯ \u{a0}Press up to edit queued messages\n\
         ────────────────────────────\n  status"
    );
    fs::write(tmp.path().join("native-queue-pane"), queue).unwrap();
    let script = fs::read_to_string(&tmux).unwrap().replace(
        "if args[0] == 'capture-pane':\n",
        "if args[0] == 'capture-pane':\n    if (root / 'submitted').exists(): print((root / 'native-queue-pane').read_text()); sys.exit(0)\n",
    );
    fs::write(&tmux, script).unwrap();
    let st = state(tmp.path(), Some(TOKEN), tmux);
    let record = load_session_record(&st.context, ID).unwrap();
    activity::activate_runtime(&st.context, &record).unwrap();
    activity::ingest_event(&st.context, ID, busy_event(&launch, "claude")).unwrap();
    assert_working(&st);
    let uri = format!("/sessions/{ID}/prompt/v2");

    // The explicitly requested idle-only policy remains distinct and terminal.
    let mut idle_only = body(&launch, "idle-only");
    idle_only["require_idle"] = json!(true);
    let (status, refused) = call(router(st.clone()), post_json(&uri, Some(TOKEN), idle_only)).await;
    assert_eq!(status, StatusCode::CONFLICT, "{refused}");
    assert_eq!(refused["error"]["code"], "busy");
    assert!(!tmp.path().join("submitted").exists());

    // Immediate handoff still requires the exact runtime and an empty composer.
    let (status, refused) = call(
        router(st.clone()),
        post_json(&uri, Some(TOKEN), body("replaced-runtime", "stale-runtime")),
    )
    .await;
    assert_eq!(status, StatusCode::CONFLICT, "{refused}");
    assert_eq!(refused["error"]["code"], "session-incarnation-conflict");
    fs::write(tmp.path().join("composer"), "owner draft").unwrap();
    let (status, refused) = call(
        router(st.clone()),
        post_json(&uri, Some(TOKEN), body(&launch, "draft-refusal")),
    )
    .await;
    assert_eq!(status, StatusCode::CONFLICT, "{refused}");
    assert_eq!(refused["error"]["code"], "composer-not-empty");
    assert_eq!(
        fs::read_to_string(tmp.path().join("composer")).unwrap(),
        "owner draft"
    );
    assert!(!tmp.path().join("submitted").exists());
    fs::write(tmp.path().join("composer"), "").unwrap();

    let (status, result) = call(
        router(st.clone()),
        post_json(&uri, Some(TOKEN), body(&launch, "busy-reply")),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{result}");
    assert_eq!(result["data"]["queued"], true);
    assert_eq!(result["data"]["receipt"]["outcome"], "submitted");
    assert_eq!(
        fs::read_to_string(tmp.path().join("submitted")).unwrap(),
        TEXT
    );
    assert_working(&st);
    replay_and_lookup(st, &launch, &result).await;
    assert_eq!(
        fs::read_to_string(tmp.path().join("enter-count")).unwrap(),
        "1"
    );
}
