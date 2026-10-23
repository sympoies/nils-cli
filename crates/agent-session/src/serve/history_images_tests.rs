use super::*;
use axum::http::Request;
use pretty_assertions::assert_eq;
use tower::ServiceExt;

const PNG: &str =
    "iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAQAAAC1HAwCAAAAC0lEQVR42mP8/x8AAwMCAO+aC1cAAAAASUVORK5CYII=";

fn request(uri: &str, authenticated: bool) -> Request<Body> {
    let builder = Request::builder().uri(uri);
    let builder = if authenticated {
        builder.header(AUTHORIZATION, "Bearer test-image-token")
    } else {
        builder
    };
    builder.body(Body::empty()).unwrap()
}

#[tokio::test]
async fn history_images_are_authenticated_bounded_and_bound_to_the_source_row() {
    use base64::Engine;
    let temp = tempfile::TempDir::new().unwrap();
    let cwd = temp.path().join("repo");
    fs::create_dir(&cwd).unwrap();
    let root = temp.path().join("history");
    fs::create_dir(&root).unwrap();
    let file = root.join("rollout.jsonl");
    let row = json!({"type":"response_item","payload":{"type":"message","role":"user","content":[
        {"type":"input_text","text":"image fixture"},
        {"type":"input_image","image_url":format!("data:image/png;base64,{PNG}")},
        {"type":"input_image","path":cwd.join("screen.png")},
        {"type":"input_image","image_url":"https://example.invalid/private.png"},
        {"type":"input_image","path":"../escape.png"},
        {"type":"input_image","path":"link.png"}
    ],"internal_chat_message_metadata_passthrough":{"content_item_kinds":["user.text","user.image"]}}});
    let meta = json!({"type":"session_meta","payload":{"id":"image-session","cwd":cwd,"timestamp":"2026-10-16T00:00:00Z","source":"cli"}});
    fs::write(&file, format!("{meta}\n{row}\n")).unwrap();
    let png = base64::engine::general_purpose::STANDARD
        .decode(PNG)
        .unwrap();
    fs::write(cwd.join("screen.png"), &png).unwrap();
    std::os::unix::fs::symlink(cwd.join("screen.png"), cwd.join("link.png")).unwrap();
    let mut state = super::super::tests::state(
        temp.path(),
        Some("test-image-token"),
        PathBuf::from("/unused-tmux"),
    );
    Arc::get_mut(&mut state).unwrap().history_catalog = Arc::new(HistoryCatalog::new(
        vec![provider_history::HistorySource {
            provider: "codex".into(),
            agent_profile: None,
            root,
        }],
        provider_history::archive_root(temp.path()),
        provider_history::star_root(temp.path()),
    ));
    let history = provider_history::stable_history_id("codex", None, "image-session");
    let page = state
        .history_catalog
        .messages(&history, None, 50, HistoryMessageDirection::Latest)
        .unwrap();
    assert_eq!(page.messages.len(), 1);
    assert_eq!(page.messages[0].images.len(), 5);
    let uri = |index: usize| {
        format!(
            "/history/sessions/{history}/images/{}",
            page.messages[0].images[index].id
        )
    };
    let app = super::super::router(state.clone());
    assert_eq!(
        app.clone()
            .oneshot(request(&uri(0), false))
            .await
            .unwrap()
            .status(),
        StatusCode::UNAUTHORIZED
    );
    for index in [0, 1] {
        let response = app
            .clone()
            .oneshot(request(&uri(index), true))
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        assert_eq!(response.headers()[CONTENT_TYPE], "image/png");
        assert_eq!(response.headers()["cache-control"], "private, no-store");
        assert_eq!(response.headers()["x-content-type-options"], "nosniff");
        assert_eq!(
            axum::body::to_bytes(response.into_body(), 1024)
                .await
                .unwrap()
                .as_ref(),
            png
        );
    }
    for index in [2, 3, 4] {
        let response = app
            .clone()
            .oneshot(request(&uri(index), true))
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::UNPROCESSABLE_ENTITY);
        let body = axum::body::to_bytes(response.into_body(), 1024)
            .await
            .unwrap();
        assert_eq!(
            serde_json::from_slice::<Value>(&body).unwrap()["error"]["code"],
            "history-image-unavailable"
        );
    }
    fs::remove_file(cwd.join("screen.png")).unwrap();
    let response = app.clone().oneshot(request(&uri(1), true)).await.unwrap();
    assert_eq!(response.status(), StatusCode::NOT_FOUND);
    let body = axum::body::to_bytes(response.into_body(), 1024)
        .await
        .unwrap();
    assert_eq!(
        serde_json::from_slice::<Value>(&body).unwrap()["error"]["code"],
        "history-image-deleted"
    );
    // A rewrite at the same offset must not serve a different image under a stale ID.
    fs::write(
        &file,
        format!("{meta}\n{}\n", row.to_string().replace(PNG, "AAAA")),
    )
    .unwrap();
    assert_eq!(
        app.clone()
            .oneshot(request(&uri(0), true))
            .await
            .unwrap()
            .status(),
        StatusCode::NOT_FOUND
    );
    fs::remove_file(&file).unwrap();
    assert_eq!(
        app.oneshot(request(&uri(0), true)).await.unwrap().status(),
        StatusCode::NOT_FOUND
    );
}

#[tokio::test]
async fn history_parts_are_opt_in_and_keep_provider_item_identity() {
    let temp = tempfile::TempDir::new().unwrap();
    let root = temp.path().join("history");
    let transcript_dir = root.join("2026/10/23");
    fs::create_dir_all(&transcript_dir).unwrap();
    fs::write(
        transcript_dir.join("rollout.jsonl"),
        concat!(
            "{\"timestamp\":\"2026-10-23T00:00:00Z\",\"type\":\"session_meta\",\"payload\":{\"id\":\"parts-session\",\"cwd\":\"/workspace/fixture\",\"source\":\"cli\",\"cli_version\":\"0.134.0\",\"timestamp\":\"2026-10-23T00:00:00Z\"}}\n",
            "{\"timestamp\":\"2026-10-23T00:00:01Z\",\"type\":\"response_item\",\"payload\":{\"id\":\"item-1\",\"type\":\"message\",\"role\":\"assistant\",\"content\":[{\"type\":\"output_text\",\"text\":\"hello\"}]}}\n",
            "{\"timestamp\":\"2026-10-23T00:00:02Z\",\"type\":\"response_item\",\"payload\":{\"id\":\"call-item\",\"call_id\":\"tool-1\",\"type\":\"function_call\",\"name\":\"Read\",\"arguments\":\"{\\\"file_path\\\":\\\"src/config.rs\\\"}\"}}\n",
            "{\"timestamp\":\"2026-10-23T00:00:03Z\",\"type\":\"response_item\",\"payload\":{\"id\":\"result-item\",\"call_id\":\"tool-1\",\"type\":\"function_call_output\",\"output\":\"read complete\"}}\n"
        ),
    )
    .unwrap();
    let mut state = super::super::tests::state(
        temp.path(),
        Some("test-image-token"),
        PathBuf::from("/unused-tmux"),
    );
    Arc::get_mut(&mut state).unwrap().history_catalog = Arc::new(HistoryCatalog::new(
        vec![provider_history::HistorySource {
            provider: "codex".into(),
            agent_profile: None,
            root,
        }],
        provider_history::archive_root(temp.path()),
        provider_history::star_root(temp.path()),
    ));
    let history = provider_history::stable_history_id("codex", None, "parts-session");
    let app = super::super::router(state);

    let opt_in = app
        .clone()
        .oneshot(request(
            &format!("/history/sessions/{history}/messages?parts=v1"),
            true,
        ))
        .await
        .unwrap();
    let body = axum::body::to_bytes(opt_in.into_body(), 4096)
        .await
        .unwrap();
    let body: Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(body["data"]["messages"][0]["text"], "hello");
    assert_eq!(body["data"]["messages"].as_array().unwrap().len(), 3);
    assert_eq!(
        body["data"]["messages"][0]["parts"][0],
        json!({
            "kind": "text",
            "id": "item-1",
            "timestamp": "2026-10-23T00:00:01Z",
            "text": "hello"
        })
    );
    assert_eq!(body["data"]["messages"][1]["role"], "assistant");
    assert_eq!(body["data"]["messages"][1]["parts"][0]["kind"], "tool_call");
    assert_eq!(body["data"]["messages"][1]["parts"][0]["id"], "call-item");
    assert_eq!(body["data"]["messages"][1]["parts"][0]["call_id"], "tool-1");
    assert_eq!(
        body["data"]["messages"][2]["parts"][0]["kind"],
        "tool_result"
    );
    assert_eq!(body["data"]["messages"][2]["parts"][0]["id"], "result-item");
    assert_eq!(body["data"]["messages"][2]["parts"][0]["call_id"], "tool-1");

    let default_shape = app
        .clone()
        .oneshot(request(
            &format!("/history/sessions/{history}/messages"),
            true,
        ))
        .await
        .unwrap();
    let body = axum::body::to_bytes(default_shape.into_body(), 4096)
        .await
        .unwrap();
    let body: Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(body["data"]["messages"][0]["text"], "hello");
    assert_eq!(body["data"]["messages"].as_array().unwrap().len(), 1);
    assert!(body["data"]["messages"][0].get("parts").is_none());

    let unsupported = app
        .oneshot(request(
            &format!("/history/sessions/{history}/messages?parts=v2"),
            true,
        ))
        .await
        .unwrap();
    assert_eq!(unsupported.status(), StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn history_attachment_reads_check_live_and_archived_session_identity() {
    use base64::Engine;
    let temp = tempfile::TempDir::new().unwrap();
    let state_dir = temp.path().join("state");
    let cwd = temp.path().join("repo");
    let root = temp.path().join("projects");
    fs::create_dir(&cwd).unwrap();
    fs::create_dir_all(root.join("repo")).unwrap();
    let context = crate::CliContext::resolve(Some(state_dir.clone()), None).unwrap();
    let png = base64::engine::general_purpose::STANDARD
        .decode(PNG)
        .unwrap();
    let attachment = |id: &str, name: &str| {
        state_dir
            .join("sessions")
            .join(id)
            .join("attachments")
            .join(name)
    };
    for id in ["own", "other", "unbound", "archived"] {
        fs::create_dir_all(attachment(id, "image.png").parent().unwrap()).unwrap();
        fs::write(attachment(id, "image.png"), &png).unwrap();
        let mut record =
            super::super::tests::provider_discovery_record(id, "fixture-tmux", "fixture-launch", 1);
        record.cwd = cwd.to_string_lossy().into_owned();
        if id == "unbound" {
            record.provider_resume = None;
        }
        crate::write_session_record(&context, &record).unwrap();
    }
    std::os::unix::fs::symlink(
        attachment("own", "image.png"),
        attachment("own", "link.png"),
    )
    .unwrap();
    let references = [
        attachment("own", "image.png"),
        attachment("other", "image.png"),
        attachment("unbound", "image.png"),
        attachment("own", "link.png"),
        state_dir.join("sessions/own/session.json"),
        attachment("archived", "image.png"),
    ];
    let content: Vec<_> = references
        .iter()
        .map(|path| json!({"type":"image","source":{"type":"file","path":path}}))
        .collect();
    let row = json!({"sessionId":"own-provider","cwd":cwd,"type":"user","promptSource":"typed","message":{"role":"user","content":content}});
    fs::write(root.join("repo/own-provider.jsonl"), format!("{row}\n")).unwrap();
    let mut state = super::super::tests::state(
        &state_dir,
        Some("test-image-token"),
        PathBuf::from("/unused-tmux"),
    );
    Arc::get_mut(&mut state).unwrap().history_catalog = Arc::new(HistoryCatalog::new(
        vec![provider_history::HistorySource {
            provider: "claude".into(),
            agent_profile: None,
            root,
        }],
        provider_history::archive_root(&state_dir),
        provider_history::star_root(&state_dir),
    ));
    let history = provider_history::stable_history_id("claude", None, "own-provider");
    let page = state
        .history_catalog
        .messages(&history, None, 50, HistoryMessageDirection::Latest)
        .unwrap();
    assert_eq!(page.messages[0].images.len(), 6);
    let uri = |index: usize| {
        format!(
            "/history/sessions/{history}/images/{}",
            page.messages[0].images[index].id
        )
    };
    let app = super::super::router(state);
    for (index, expected) in [
        (0, StatusCode::OK),
        (1, StatusCode::NOT_FOUND),
        (2, StatusCode::NOT_FOUND),
        (3, StatusCode::UNPROCESSABLE_ENTITY),
        (4, StatusCode::UNPROCESSABLE_ENTITY),
        (5, StatusCode::NOT_FOUND),
    ] {
        let response = app
            .clone()
            .oneshot(request(&uri(index), true))
            .await
            .unwrap();
        assert_eq!(response.status(), expected, "attachment index {index}");
        assert_eq!(response.headers()["cache-control"], "private, no-store");
    }
    let archives = provider_history::archive_root(&state_dir);
    let mut archive = provider_history::ArchivedSession {
        title_mode: Default::default(),
        schema_version: "agent-session.history-archive.v1".into(),
        history_id: history.clone(),
        provider: "claude".into(),
        provider_session_id: "own-provider".into(),
        agent_profile: None,
        session_id: Some("archived".into()),
        title: None,
        title_state: None,
        cwd: cwd.to_string_lossy().into_owned(),
        created_at: "2026-10-16T00:00:00Z".into(),
        updated_at: "2026-10-16T00:00:00Z".into(),
        archived_at: "2026-10-16T00:00:00Z".into(),
    };
    fs::remove_file(state_dir.join("sessions/archived/session.json")).unwrap();
    provider_history::write_archive(&archives, &archive)
        .unwrap()
        .commit();
    assert_eq!(
        app.clone()
            .oneshot(request(&uri(5), true))
            .await
            .unwrap()
            .status(),
        StatusCode::OK
    );
    archive.session_id = Some("different-session".into());
    provider_history::write_archive(&archives, &archive)
        .unwrap()
        .commit();
    assert_eq!(
        app.oneshot(request(&uri(5), true)).await.unwrap().status(),
        StatusCode::NOT_FOUND
    );
}
