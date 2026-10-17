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
