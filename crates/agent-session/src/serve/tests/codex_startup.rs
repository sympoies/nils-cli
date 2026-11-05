//! HTTP create followed by the actual Unix/WebSocket proxy; no live provider.
use super::super::*;
use pretty_assertions::assert_eq;

#[tokio::test]
async fn serve_created_codex_without_prompt_settles_from_primary_idle_response() {
    for status in [Some("active"), None, Some("idle")] {
        created_status(status).await;
    }
}

async fn created_status(provider_status: Option<&'static str>) {
    let lock = GlobalStateLock::new();
    let _without_broker = EnvGuard::remove(&lock, "AGENT_SESSION_CODEX_ACCOUNT_BROKER");
    let tmp = tempfile::tempdir().unwrap();
    let cwd = tmp.path().join("non-repository");
    fs::create_dir(&cwd).unwrap();
    let runtime_dir = tempfile::tempdir_in("/tmp").unwrap();
    fs::set_permissions(runtime_dir.path(), fs::Permissions::from_mode(0o700)).unwrap();
    let codex = executable(
        &tmp.path().join("codex"),
        "#!/bin/sh\ncase \"$*\" in\n--version) echo 'codex-cli 0.162.1';;\n'app-server --help') echo '--listen <URL> unix://';;\n*) exit 1;;\nesac\n",
    );
    let _codex = EnvGuard::set(&lock, "AGENT_SESSION_CODEX_BIN", codex.to_str().unwrap());
    let _runtime = EnvGuard::set(
        &lock,
        "XDG_RUNTIME_DIR",
        runtime_dir.path().to_str().unwrap(),
    );
    let _mode = EnvGuard::set(&lock, "AGENT_SESSION_CODEX_RUNTIME", "app-server");
    let _capture = EnvGuard::set(&lock, "AGENT_SESSION_CODEX_CAPTURE_TIMEOUT_MS", "10");
    let st = state(tmp.path(), Some(TOKEN), minimal_tmux(tmp.path()));
    let (status, body) = call(
        router(st.clone()),
        post_json(
            "/sessions",
            Some(TOKEN),
            json!({
                "agent":"codex", "id":"fresh-idle", "cwd":cwd, "origin":{"parent":null}
            }),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let record = load_session_record(&st.context, "fresh-idle").unwrap();
    assert!(record.agent_args.is_empty());
    assert!(record.prompt_file.is_none());
    assert!(crate::origin::Fields::of(&record).origin.parent.is_none());
    assert_eq!(
        activity::state_for_view(&st.context, &record)
            .unwrap()
            .phase,
        activity::TurnPhase::Starting
    );
    let runtime = record.runtime.as_ref().unwrap();
    let upstream = PathBuf::from(
        runtime.extra[codex_app_server::SOCKET_KEY]
            .as_str()
            .unwrap(),
    );
    let listen = PathBuf::from(runtime.extra[codex_app_server::PROXY_KEY].as_str().unwrap());
    let listener = tokio::net::UnixListener::bind(&upstream).unwrap();
    let cx = st.context.clone();
    let args = crate::cli::CodexAppServerProxyArgs {
        id: record.id.clone(),
        upstream,
        listen: listen.clone(),
    };
    let proxy = tokio::task::spawn_blocking(move || codex_app_server::run_proxy(&cx, args));
    let peer = tokio::spawn(async move {
        let (stream, _) = listener.accept().await.unwrap();
        let mut ws = tokio_tungstenite::accept_async(stream).await.unwrap();
        let request = ws.next().await.unwrap().unwrap();
        let request: Value = serde_json::from_str(request.to_text().unwrap()).unwrap();
        assert_eq!(request["method"], "thread/start");
        let mut response = json!({"id":request["id"],"result":{
            "model":"provider-default", "thread":{"id":"primary-thread"}
        }});
        if let Some(status) = provider_status {
            response["result"]["thread"]["status"] = json!({"type":status});
        }
        ws.send(ClientMessage::Text(response.to_string().into()))
            .await
            .unwrap();
        while let Some(Ok(message)) = ws.next().await {
            if message.is_close() {
                break;
            }
        }
    });
    let stream = loop {
        if let Ok(stream) = tokio::net::UnixStream::connect(&listen).await {
            break stream;
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    };
    let (mut tui, _) = tokio_tungstenite::client_async("ws://localhost", stream)
        .await
        .unwrap();
    tui.send(ClientMessage::Text(
        json!({"id":1,"method":"thread/start","params":{"cwd":cwd}})
            .to_string()
            .into(),
    ))
    .await
    .unwrap();
    let response = tui.next().await.unwrap().unwrap();
    let response: Value = serde_json::from_str(response.to_text().unwrap()).unwrap();
    assert_eq!(
        response["result"]["thread"]["status"]["type"].as_str(),
        provider_status
    );
    // The acknowledgement fences observation before forwarding to the composer.
    let turn = activity::state_for_view(&st.context, &record).unwrap();
    tui.close(None).await.unwrap();
    peer.await.unwrap();
    assert_eq!(proxy.await.unwrap(), 0);
    let expected = if provider_status == Some("idle") {
        activity::TurnPhase::Waiting
    } else {
        activity::TurnPhase::Starting
    };
    assert_eq!(turn.phase, expected);
    if provider_status == Some("idle") {
        assert_eq!(
            serde_json::to_value(turn.source).unwrap()["kind"],
            "provider_hook"
        );
        let (_, view) = call(router(st.clone()), get("/sessions")).await;
        assert_eq!(
            view["data"]["sessions"][0]["turn_state"]["phase"],
            "waiting"
        );
    }
    assert!(turn.current_turn.is_none());
    assert!(turn.last_turn.is_none());
}
