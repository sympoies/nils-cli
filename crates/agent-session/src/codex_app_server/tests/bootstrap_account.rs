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

#[tokio::test]
async fn replacement_account_read_uses_binding_current_at_request_not_proxy_launch() {
    let lock = GlobalStateLock::new();
    let _broker = EnvGuard::set(
        &lock,
        "AGENT_SESSION_CODEX_ACCOUNT_BROKER",
        r#"["/configured/broker"]"#,
    );
    let tmp = tempfile::tempdir().unwrap();
    let (_socket_dir, path) = nils_test_support::short_socket_path("current-account.sock");
    let listener = UnixListener::bind(&path).unwrap();
    let context = CliContext {
        state_dir: tmp.path().join("state"),
        host: None,
    };
    let mut record = record_with_runtime("current-account", &path);
    record.runtime.as_mut().unwrap().generation = 2;
    record.extra.insert(
        "codex_account_binding".into(),
        json!({
            "schema_version":crate::codex_account::BINDING_SCHEMA_VERSION,
            "selected_account":"latest-account", "revision":2, "state":"pending",
            "selection_source":"explicit", "updated_at":"2030-01-01T00:00:00Z"
        }),
    );
    fs::create_dir_all(crate::session_dir(&context, &record.id)).unwrap();
    crate::write_session_record(&context, &record).unwrap();
    crate::activity::activate_runtime(&context, &record).unwrap();
    let server_context = context.clone();
    let server_record = record.clone();
    let server = tokio::spawn(async move {
        let (stream, _) = listener.accept().await.unwrap();
        let mut socket = tokio_tungstenite::accept_async(stream).await.unwrap();
        // Valid native models response; discovery also proves proxy startup.
        let models = receive_json(&mut socket).await;
        assert_eq!(models["method"], "model/list");
        respond(&mut socket, &models, json!({"data":[],"nextCursor":null})).await;
        for _ in 0..2 {
            let request = receive_json(&mut socket).await;
            assert_eq!(request["method"], "account/read");
            let current = crate::load_session_record(&server_context, &server_record.id).unwrap();
            assert!(crate::codex_account::ensure_terminal_input_allowed(&current).is_ok());
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
    send_json(&mut tui, json!({"id":1,"method":"model/list","params":{}}))
        .await
        .unwrap();
    assert_eq!(receive_json(&mut tui).await["result"]["data"], json!([]));

    // Initial application and a same-account refresh complete before account/read.
    let launch_id = &record.runtime.as_ref().unwrap().launch_id;
    crate::codex_account::finish_binding(
        &context,
        &record.id,
        launch_id,
        "latest-account",
        2,
        Ok(()),
    )
    .unwrap();
    send_json(
        &mut tui,
        json!({"id":2,"method":"account/read","params":{"refreshToken":false}}),
    )
    .await
    .unwrap();
    assert_eq!(
        receive_json(&mut tui).await["result"]["account"]["type"],
        "chatgpt"
    );
    let attempt = crate::codex_account::begin_refresh_binding(
        &context,
        &record.id,
        launch_id,
        "latest-account",
    )
    .unwrap();
    crate::codex_account::finish_binding(
        &context,
        &record.id,
        launch_id,
        "latest-account",
        attempt.revision(),
        Ok(()),
    )
    .unwrap();
    send_json(
        &mut tui,
        json!({"id":3,"method":"account/read","params":{"refreshToken":false}}),
    )
    .await
    .unwrap();
    let result = receive_json(&mut tui).await;
    assert_eq!(result["result"]["account"]["type"], "chatgpt", "{result}");
    server.await.unwrap();
    let _ = proxy.await.unwrap();
}

#[tokio::test]
async fn bootstrap_account_read_keeps_runtime_account_and_revision_wait_fences() {
    for replacement in ["runtime", "account", "revision"] {
        let tmp = tempfile::tempdir().unwrap();
        let context = CliContext {
            state_dir: tmp.path().join("state"),
            host: None,
        };
        let mut record = record_with_runtime("account-wait-fence", &tmp.path().join("upstream"));
        record.extra.insert(
            "codex_account_binding".into(),
            json!({
                "schema_version":crate::codex_account::BINDING_SCHEMA_VERSION,
                "selected_account":"latest-account", "revision":2, "state":"pending",
                "selection_source":"explicit", "updated_at":"2030-01-01T00:00:00Z"
            }),
        );
        fs::create_dir_all(crate::session_dir(&context, &record.id)).unwrap();
        crate::write_session_record(&context, &record).unwrap();
        let mut bootstrap = FreshBootstrap::for_runtime(&context, &record);
        let waiting =
            wait_for_bootstrap_binding(&context, &record, Some("account/read"), &mut bootstrap);
        tokio::pin!(waiting);
        assert!(
            tokio::time::timeout(Duration::from_millis(100), &mut waiting)
                .await
                .is_err()
        );
        let mut current = record.clone();
        match replacement {
            "runtime" => current.runtime.as_mut().unwrap().launch_id = "replacement-runtime".into(),
            "account" => {
                current.extra.get_mut("codex_account_binding").unwrap()["selected_account"] =
                    json!("other-account")
            }
            "revision" => {
                current.extra.get_mut("codex_account_binding").unwrap()["revision"] = json!(3)
            }
            _ => unreachable!(),
        }
        let binding = current.extra.get_mut("codex_account_binding").unwrap();
        binding["state"] = json!("bound");
        binding["applied_runtime_id"] = json!(current.runtime.as_ref().unwrap().launch_id);
        crate::write_session_record(&context, &current).unwrap();
        let rejection = tokio::time::timeout(Duration::from_secs(5), &mut waiting)
            .await
            .unwrap()
            .unwrap_err();
        let response = message_value(&tui_busy_response(&json!(1), rejection))
            .unwrap()
            .unwrap();
        assert_eq!(
            response["error"]["data"]["reason"], "account_authority_unavailable",
            "{replacement}"
        );
        assert_eq!(
            response["error"]["data"]["cause"]["code"],
            if replacement == "runtime" {
                "initial-binding-runtime-changed"
            } else {
                "initial-binding-replaced"
            },
            "{replacement}"
        );
        assert_eq!(response["error"]["data"]["retryable"], false);
        assert_eq!(
            response["error"]["data"]["recovery"]["outcome"],
            "not_forwarded"
        );
    }
}
