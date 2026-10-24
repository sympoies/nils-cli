use super::*;
use axum::http::Request;
use base64::Engine;
use pretty_assertions::assert_eq;
use tower::ServiceExt;

const TOKEN: &str = "fixture-image-token";
const PNG: &str =
    "iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAQAAAC1HAwCAAAAC0lEQVR42mP8/x8AAwMCAO+aC1cAAAAASUVORK5CYII=";

struct Fixture {
    _temp: tempfile::TempDir,
    state: Arc<ServeState>,
    transcript: PathBuf,
    history: String,
    png: Vec<u8>,
}

impl Fixture {
    fn new() -> Self {
        let temp = tempfile::TempDir::new().unwrap();
        let state_dir = temp.path().join("state");
        let cwd = temp.path().to_path_buf();
        let root = temp.path().join("history");

        fs::create_dir_all(&root).unwrap();
        let mut state =
            super::super::tests::state(&state_dir, Some(TOKEN), PathBuf::from("/unused-tmux"));
        for id in ["own", "other"] {
            let mut record = super::super::tests::provider_discovery_record(
                id,
                "fixture-runtime",
                "fixture-launch",
                1,
            );
            record.agent = "codex".into();
            record.cwd = cwd.to_string_lossy().into_owned();
            record.provider_resume.as_mut().unwrap().provider = "codex".into();
            crate::write_session_record(&state.context, &record).unwrap();
        }
        Arc::get_mut(&mut state).unwrap().history_catalog = Arc::new(HistoryCatalog::new(
            vec![provider_history::HistorySource {
                provider: "codex".into(),
                agent_profile: None,
                root: root.clone(),
            }],
            provider_history::archive_root(&state_dir),
            provider_history::star_root(&state_dir),
        ));
        let transcript = root.join("rollout.jsonl");
        let meta = json!({"type":"session_meta","payload":{"id":"own-provider","cwd":cwd,
            "timestamp":"2026-10-16T00:00:00Z","source":"cli","cli_version":"0.134.0"}});
        fs::write(&transcript, format!("{meta}\n")).unwrap();
        Self {
            _temp: temp,
            state,
            transcript,
            history: provider_history::stable_history_id("codex", None, "own-provider"),
            png: base64::engine::general_purpose::STANDARD
                .decode(PNG)
                .unwrap(),
        }
    }

    fn append(&self, content: Value) {
        use std::io::Write;
        let row = json!({"type":"response_item","payload":{"id":"fixture-message","type":"message",
            "role":"user","content":content,
            "internal_chat_message_metadata_passthrough":{"content_item_kinds":["user.text"]}}});
        writeln!(
            fs::OpenOptions::new()
                .append(true)
                .open(&self.transcript)
                .unwrap(),
            "{row}"
        )
        .unwrap();
    }

    async fn upload(&self, session: &str, filename: &str, bytes: &[u8]) -> PathBuf {
        let response = super::super::router(self.state.clone())
            .oneshot(
                Request::post(format!(
                    "/sessions/{session}/attachments?filename={filename}"
                ))
                .header(AUTHORIZATION, format!("Bearer {TOKEN}"))
                .body(Body::from(bytes.to_vec()))
                .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        let body = axum::body::to_bytes(response.into_body(), 4096)
            .await
            .unwrap();
        let body: Value = serde_json::from_slice(&body).unwrap();
        PathBuf::from(body["data"]["attachment"]["path"].as_str().unwrap())
    }

    async fn image(&self, id: &str) -> Response {
        super::super::router(self.state.clone())
            .oneshot(
                Request::get(format!("/history/sessions/{}/images/{id}", self.history))
                    .header(AUTHORIZATION, format!("Bearer {TOKEN}"))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap()
    }

    async fn refresh(&self) -> Value {
        let response = super::super::router(self.state.clone())
            .oneshot(
                Request::get(format!(
                    "/history/sessions/{}/messages?parts=v1",
                    self.history
                ))
                .header(AUTHORIZATION, format!("Bearer {TOKEN}"))
                .body(Body::empty())
                .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        let body = axum::body::to_bytes(response.into_body(), 65536)
            .await
            .unwrap();
        serde_json::from_slice(&body).unwrap()
    }
}

#[tokio::test]
async fn quoted_upload_history_refresh_preserves_caption_order_and_authenticated_reads() {
    let fixture = Fixture::new();
    assert_eq!(fixture.refresh().await["data"]["messages"], json!([]));
    let first = fixture.upload("own", "first.png", &fixture.png).await;
    let second = fixture.upload("own", "second.png", &fixture.png).await;
    fixture.append(json!([{"type":"input_text","text":format!(
        "Here's \"{}\" between '{}' after", first.display(), second.display())}]));
    let refreshed = fixture.refresh().await;
    let message = &refreshed["data"]["messages"][0];
    assert_eq!(message["images"].as_array().map(Vec::len), Some(2));
    assert_eq!(message["text"], "Here's \n between \n after");
    let parts = message["parts"].as_array().unwrap();
    assert_eq!(
        parts
            .iter()
            .map(|p| p["kind"].as_str().unwrap())
            .collect::<Vec<_>>(),
        ["text", "image", "text", "image", "text"]
    );
    assert_eq!(parts[0]["text"], "Here's");
    assert_eq!(parts[2]["text"], "between");
    assert_eq!(parts[4]["text"], "after");
    for (index, image) in message["images"].as_array().unwrap().iter().enumerate() {
        assert_eq!(parts[index * 2 + 1]["ref"], image["id"]);
        let uri = format!(
            "/history/sessions/{}/images/{}",
            fixture.history,
            image["id"].as_str().unwrap()
        );
        let app = super::super::router(fixture.state.clone());
        let denied = app
            .clone()
            .oneshot(Request::get(&uri).body(Body::empty()).unwrap())
            .await
            .unwrap();
        assert_eq!(denied.status(), StatusCode::UNAUTHORIZED);
        let response = app
            .oneshot(
                Request::get(&uri)
                    .header(AUTHORIZATION, format!("Bearer {TOKEN}"))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        assert_eq!(response.headers()[CONTENT_TYPE], "image/png");
        assert_eq!(response.headers()["cache-control"], "private, no-store");
        assert_eq!(
            axum::body::to_bytes(response.into_body(), 4096)
                .await
                .unwrap()
                .as_ref(),
            fixture.png
        );
    }
    let page = fixture
        .state
        .history_catalog
        .messages(&fixture.history, None, 50, HistoryMessageDirection::Forward)
        .unwrap();
    assert_eq!(page.messages[0].images.len(), 2);
    assert_eq!(page.messages[0].text, message["text"].as_str().unwrap());
}

#[tokio::test]
async fn quoted_upload_history_leaves_untrusted_and_non_image_paths_as_text() {
    let fixture = Fixture::new();
    let own = fixture.upload("own", "valid.png", &fixture.png).await;
    let other = fixture.upload("other", "other.png", &fixture.png).await;
    let text_file = fixture.upload("own", "notes.txt", b"caption").await;
    let fake = fixture.upload("own", "fake.png", b"not an image").await;
    let outside = fixture._temp.path().join("outside.png");
    fs::write(&outside, &fixture.png).unwrap();
    let linked = own.parent().unwrap().join("linked.png");
    std::os::unix::fs::symlink(&outside, &linked).unwrap();
    let traversal = own
        .parent()
        .unwrap()
        .join("../attachments")
        .join(own.file_name().unwrap());
    let paths = [other, text_file, fake, outside, linked, traversal];
    let text = paths
        .iter()
        .map(|path| format!("\"{}\"", path.display()))
        .collect::<Vec<_>>()
        .join("\n");
    fixture.append(json!([{"type":"input_text","text":text}]));
    let refreshed = fixture.refresh().await;
    let message = &refreshed["data"]["messages"][0];
    assert!(message.get("images").is_none());
    assert_eq!(message["text"], text);
    assert_eq!(message["parts"].as_array().unwrap().len(), 1);
}

#[tokio::test]
async fn quoted_upload_history_selectors_recheck_ownership_without_shifting_after_deletion() {
    let fixture = Fixture::new();
    let first = fixture.upload("own", "first.png", &fixture.png).await;
    let second = fixture.upload("own", "second.png", &fixture.png).await;
    let outside = fixture.upload("other", "outside.png", &fixture.png).await;
    let text = format!(
        "\"{}\" \"{}\" \"{}\"",
        outside.display(),
        first.display(),
        second.display()
    );
    fixture.append(json!([{"type":"input_text","text":text}]));
    let page = fixture.refresh().await;
    let images = page["data"]["messages"][0]["images"].as_array().unwrap();
    assert_eq!(images.len(), 2);
    let first_id = images[0]["id"].as_str().unwrap();
    let second_id = images[1]["id"].as_str().unwrap();
    // A forged selector to another session's quoted image still cannot read it.
    let mut forged: Vec<_> = first_id.split('.').collect();
    forged[2] = "u0-0";
    assert_eq!(
        fixture.image(&forged.join(".")).await.status(),
        StatusCode::NOT_FOUND
    );
    fs::remove_file(&first).unwrap();
    assert_eq!(
        fixture.image(first_id).await.status(),
        StatusCode::NOT_FOUND
    );
    assert_eq!(fixture.image(second_id).await.status(), StatusCode::OK);
    let refreshed = fixture.refresh().await;
    assert_eq!(
        refreshed["data"]["messages"][0]["images"],
        json!([{"id":second_id}])
    );
    // Swapping a previously accepted file for a symlink cannot escape the upload root.
    std::os::unix::fs::symlink(&outside, &first).unwrap();
    assert_eq!(
        fixture.image(first_id).await.status(),
        StatusCode::UNPROCESSABLE_ENTITY
    );
}

#[tokio::test]
async fn quoted_upload_history_keeps_structured_and_markdown_images_and_image_only_turns() {
    let fixture = Fixture::new();
    let upload = fixture.upload("own", "image.png", &fixture.png).await;
    fixture.append(json!([
        {"type":"input_image","image_url":format!("data:image/png;base64,{PNG}")},
        {"type":"input_text","text":format!("![before](data:image/png;base64,{PNG}) \"{}\"", upload.display())},
        {"type":"input_image","image_url":format!("data:image/png;base64,{PNG}")}
    ]));
    fixture.append(json!([{"type":"input_text","text":format!("'{}'", upload.display())}]));
    let page = fixture.refresh().await;
    let messages = page["data"]["messages"].as_array().unwrap();
    assert_eq!(messages.len(), 2);
    let images = messages[0]["images"].as_array().unwrap();
    assert_eq!(images.len(), 4);
    let selectors: Vec<_> = images
        .iter()
        .map(|image| image["id"].as_str().unwrap().split('.').nth(2).unwrap())
        .collect();
    assert_eq!(selectors[0], "0");
    assert_eq!(selectors[1], "1");
    assert!(selectors[2].starts_with('u'));
    assert_eq!(selectors[3], "2");
    let refs: Vec<_> = messages[0]["parts"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|part| part["kind"] == "image")
        .map(|part| part["ref"].clone())
        .collect();
    assert_eq!(
        refs,
        images
            .iter()
            .map(|image| image["id"].clone())
            .collect::<Vec<_>>()
    );
    for image in images {
        assert_eq!(
            fixture.image(image["id"].as_str().unwrap()).await.status(),
            StatusCode::OK
        );
    }
    assert_eq!(messages[1]["text"], "");
    assert_eq!(messages[1]["images"].as_array().unwrap().len(), 1);
    assert_eq!(messages[1]["parts"][0]["kind"], "image");
}

#[path = "download_filename_tests.rs"]
mod download_filename_tests;
