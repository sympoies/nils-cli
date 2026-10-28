use super::*;
use pretty_assertions::assert_eq;

#[tokio::test]
async fn replacement_tui_account_read_waits_for_queued_account_binding() {
    let tmp = tempfile::tempdir().unwrap();
    let (_socket_dir, path) = nils_test_support::short_socket_path("account-bootstrap.sock");
    let listener = UnixListener::bind(&path).unwrap();
    let context = CliContext {
        state_dir: tmp.path().join("state"),
        host: None,
    };
    let mut record = record_with_runtime("account-bootstrap", &path);
    record.provider_resume = Some(ProviderResume {
        provider: "codex".into(),
        session_id: "original-thread".into(),
        captured_at: "2030-01-01T00:00:00Z".into(),
        capture_method: "fixture".into(),
        resume_args: canonical_provider_resume_args(
            AgentKind::Codex,
            &record.cwd,
            "original-thread",
        )
        .unwrap(),
        extra: BTreeMap::new(),
    });
    record.extra.insert(
        "codex_account_binding".into(),
        json!({
            "schema_version":crate::codex_account::BINDING_SCHEMA_VERSION,
            "selected_account":"latest-account", "revision":4, "state":"pending",
            "selection_source":"explicit", "updated_at":"2030-01-01T00:00:00Z"
        }),
    );
    fs::create_dir_all(crate::session_dir(&context, &record.id)).unwrap();
    crate::write_session_record(&context, &record).unwrap();
    crate::activity::activate_runtime(&context, &record).unwrap();
    let server_context = context.clone();
    let server_id = record.id.clone();
    let server = tokio::spawn(async move {
        let (stream, _) = listener.accept().await.unwrap();
        let mut socket = tokio_tungstenite::accept_async(stream).await.unwrap();
        let request = receive_json(&mut socket).await;
        assert_eq!(request["method"], "account/read");
        let current = crate::load_session_record(&server_context, &server_id).unwrap();
        if crate::codex_account::ensure_terminal_input_allowed(&current).is_err() {
            send_json(&mut socket, json!({"id":request["id"],"error":{"code":-32603,"message":"account changed during workspace routing discovery"}})).await.unwrap();
        } else {
            respond(
                &mut socket,
                &request,
                json!({"account":{"type":"chatgpt"},"requiresOpenaiAuth":true}),
            )
            .await;
        }
        socket.close(None).await.unwrap();
    });
    let args = crate::cli::CodexAppServerProxyArgs {
        id: record.id.clone(),
        upstream: path.clone(),
        listen: path.with_extension("proxy"),
    };
    let proxy_context = context.clone();
    let proxy_path = args.listen.clone();
    let proxy = tokio::spawn(async move { run_proxy_session(proxy_context, args).await });
    let stream = connect_socket(&proxy_path).await.unwrap();
    let (mut tui, _) = tokio_tungstenite::client_async("ws://localhost", stream)
        .await
        .unwrap();
    send_json(
        &mut tui,
        json!({"id":1,"method":"account/read","params":{"refreshToken":false}}),
    )
    .await
    .unwrap();
    tokio::time::sleep(Duration::from_millis(100)).await;
    let binding = record.extra.get_mut("codex_account_binding").unwrap();
    binding["state"] = json!("bound");
    binding["applied_runtime_id"] = json!(record.runtime.as_ref().unwrap().launch_id);
    crate::write_session_record(&context, &record).unwrap();
    let result = receive_json(&mut tui).await;
    assert_eq!(result["result"]["account"]["type"], "chatgpt", "{result}");
    assert_eq!(
        fs::read_to_string(thread_attached_path(&record).unwrap()).unwrap(),
        projected_thread_binding("original-thread")
    );
    server.await.unwrap();
    let _ = proxy.await.unwrap();
}
