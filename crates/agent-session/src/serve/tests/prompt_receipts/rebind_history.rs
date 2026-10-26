use super::*;
use pretty_assertions::assert_eq;

#[tokio::test]
async fn keyed_codex_history_binding_uses_rebound_provider_conversation() {
    let tmp = tempfile::TempDir::new().unwrap();
    let id = "prompt-history-rebind";
    let launch_id = seed_codex_app_server_session(tmp.path(), id);
    let st = state(tmp.path(), Some(TOKEN), empty_composer_tmux(tmp.path()));
    let session_path = tmp.path().join("sessions").join(id).join("session.json");
    let mut session_json: Value =
        serde_json::from_slice(&std::fs::read(&session_path).unwrap()).unwrap();
    session_json["provider_resume"] = json!({
        "provider": "codex",
        "session_id": "thread-a",
        "captured_at": "2026-10-26T00:00:00Z",
        "capture_method": "test",
        "resume_args": []
    });
    std::fs::write(
        &session_path,
        serde_json::to_vec_pretty(&session_json).unwrap(),
    )
    .unwrap();
    let outer_record = crate::load_session_record(&st.context, id).unwrap();
    let outer_thread = outer_record
        .provider_resume
        .as_ref()
        .unwrap()
        .session_id
        .clone();

    let (handle, mut commands, ready) = codex_app_server::starting_control_channel();
    st.codex_controls.lock().unwrap().insert(
        id.to_string(),
        CodexControlEntry {
            launch_id: launch_id.clone(),
            handle,
        },
    );
    let request_state = st.clone();
    let uri = format!("/sessions/{id}/prompt/v2");
    let request = post_json(
        &uri,
        Some(TOKEN),
        json!({
            "text": "continue",
            "expected_session_incarnation": launch_id,
            "request_id": "attempt-rebind",
        }),
    );
    let submission = tokio::spawn(async move { call(router(request_state), request).await });

    tokio::time::timeout(Duration::from_secs(2), async {
        while ready.receiver_count() < 3 {
            tokio::task::yield_now().await;
        }
    })
    .await
    .expect("submission should reach the app-server readiness wait");

    let mut rebound = crate::load_session_record(&st.context, id).unwrap();
    rebound.provider_resume.as_mut().unwrap().session_id = "thread-b".into();
    crate::write_session_record(&st.context, &rebound).unwrap();
    ready.send_replace(true);

    let responder = tokio::spawn(async move {
        let Some(codex_app_server::ControlCommand::Prompt { message, response }) =
            commands.recv().await
        else {
            panic!("missing accepted Codex prompt")
        };
        assert_eq!(message, "continue");
        response.send(Ok("turn-b".into())).unwrap();
    });
    let (status, receipt) = tokio::time::timeout(Duration::from_secs(5), submission)
        .await
        .expect("structured prompt response should finish")
        .unwrap();
    responder.await.unwrap();
    assert_eq!(status, StatusCode::OK, "{receipt}");
    assert_eq!(receipt["data"]["receipt"]["request_id"], "attempt-rebind");
    assert!(!receipt.to_string().contains("turn-b"));
    assert!(!receipt.to_string().contains("thread-b"));
    assert_ne!(outer_thread, "thread-b");

    let history_root = tmp.path().join("provider-history");
    std::fs::create_dir_all(&history_root).unwrap();
    let meta = json!({"type":"session_meta","payload":{"id":"thread-b","cwd":tmp.path(),"timestamp":"2026-10-26T00:00:00Z","source":"cli"}});
    let row = json!({"type":"response_item","payload":{"type":"message","role":"user","content":[{"type":"input_text","text":"continue"}],"internal_chat_message_metadata_passthrough":{"turn_id":"turn-b","content_item_kinds":["user.text"]}}});
    std::fs::write(
        history_root.join("rollout.jsonl"),
        format!("{meta}\n{row}\n"),
    )
    .unwrap();
    let catalog = crate::provider_history::HistoryCatalog::new(
        vec![crate::provider_history::HistorySource {
            provider: "codex".into(),
            agent_profile: None,
            root: history_root,
        }],
        crate::provider_history::archive_root(tmp.path()),
        crate::provider_history::star_root(tmp.path()),
    )
    .with_session_state_dir(st.context.state_dir.clone());
    let history_id = crate::provider_history::stable_history_id("codex", None, "thread-b");
    let page = catalog
        .messages(
            &history_id,
            None,
            10,
            crate::provider_history::HistoryMessageDirection::Forward,
        )
        .unwrap();

    assert_eq!(page.messages.len(), 1);
    assert_eq!(
        page.messages[0].prompt_request,
        Some(crate::provider_history::PromptRequestBinding {
            request_id: "attempt-rebind".into(),
            session_id: id.into(),
            session_incarnation: launch_id,
        })
    );
}
