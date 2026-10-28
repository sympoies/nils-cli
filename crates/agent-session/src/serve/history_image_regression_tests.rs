use super::*;
use axum::http::Request;
use pretty_assertions::assert_eq;
use tower::ServiceExt;

struct Fixture {
    _temp: tempfile::TempDir,
    state: Arc<ServeState>,
    history: String,
}

impl Fixture {
    fn new(provider: &str, rows: &str) -> Self {
        let temp = tempfile::TempDir::new().unwrap();
        let cwd = temp.path().join("workspace");
        let root = temp.path().join("history");
        fs::create_dir(&cwd).unwrap();
        fs::create_dir(&root).unwrap();
        let outside = temp.path().join("artifact.png");
        for path in [cwd.join("preview.png"), outside.clone()] {
            fs::write(path, b"\x89PNG\r\n\x1a\n").unwrap();
        }
        let rows = rows.replace("__OUTSIDE_IMAGE__", outside.to_str().unwrap());
        let rows = if provider == "codex" {
            let meta = json!({"type":"session_meta","payload":{"id":"image-fixture",
                "cwd":cwd,"source":"cli","cli_version":"0.134.0",
                "timestamp":"2026-10-01T00:00:00Z"}});
            format!("{meta}\n{rows}")
        } else {
            rows.lines()
                .map(|line| {
                    let mut row: Value = serde_json::from_str(line).unwrap();
                    row["cwd"] = json!(cwd);
                    format!("{row}\n")
                })
                .collect()
        };
        fs::write(root.join("image-fixture.jsonl"), rows).unwrap();
        let mut state = super::super::tests::state(
            temp.path(),
            Some("fixture-token"),
            PathBuf::from("/unused-tmux"),
        );
        Arc::get_mut(&mut state).unwrap().history_catalog = Arc::new(HistoryCatalog::new(
            vec![provider_history::HistorySource {
                provider: provider.into(),
                agent_profile: None,
                root,
            }],
            provider_history::archive_root(temp.path()),
            provider_history::star_root(temp.path()),
        ));
        Self {
            _temp: temp,
            state,
            history: provider_history::stable_history_id(provider, None, "image-fixture"),
        }
    }

    async fn get(&self, suffix: &str) -> Response {
        super::super::router(self.state.clone())
            .oneshot(
                Request::get(format!("/history/sessions/{}/{suffix}", self.history))
                    .header(AUTHORIZATION, "Bearer fixture-token")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap()
    }

    async fn page(&self, query: &str) -> Value {
        let response = self.get(&format!("messages{query}")).await;
        assert_eq!(response.status(), StatusCode::OK);
        serde_json::from_slice(
            &axum::body::to_bytes(response.into_body(), 16384)
                .await
                .unwrap(),
        )
        .unwrap()
    }
}

async fn assert_tool_image(provider: &str, rows: &str) {
    let fixture = Fixture::new(provider, rows);
    for query in ["", "?direction=latest", "?parts=v1"] {
        let page = fixture.page(query).await;
        let messages = page["data"]["messages"].as_array().unwrap();
        assert_eq!(messages.len(), 1, "{provider} {query}");
        assert_eq!(messages[0]["role"], "assistant", "{provider} {query}");
        let image = &messages[0]["images"][0]["id"];
        let response = fixture
            .get(&format!("images/{}", image.as_str().unwrap()))
            .await;
        assert_eq!(response.status(), StatusCode::OK);
        assert_eq!(response.headers()[CONTENT_TYPE], "image/png");
    }
    let incremental = fixture
        .state
        .history_catalog
        .incremental_messages(&fixture.history, None, 1024 * 1024)
        .unwrap();
    assert_eq!(incremental.messages.len(), 1);
    assert_eq!(incremental.messages[0].role, "assistant");
    assert!(!incremental.messages[0].human_prompt);
}

#[tokio::test]
async fn image_regression_claude_tool_result_is_assistant_output() {
    assert_tool_image(
        "claude",
        include_str!("../../tests/fixtures/history-images/claude-tool-result.jsonl"),
    )
    .await;
}

#[tokio::test]
async fn image_regression_codex_tool_result_is_fetchable_assistant_output() {
    assert_tool_image(
        "codex",
        include_str!("../../tests/fixtures/history-images/codex-tool-result.jsonl"),
    )
    .await;
}

#[tokio::test]
async fn image_regression_markdown_distinguishes_safe_file_from_boundary_refusal() {
    let fixture = Fixture::new(
        "claude",
        include_str!("../../tests/fixtures/history-images/claude-markdown.jsonl"),
    );
    let page = fixture.page("").await;
    let message = &page["data"]["messages"][0];
    assert_eq!(message["role"], "assistant");
    let images = message["images"].as_array().unwrap();
    assert_eq!(images.len(), 2);
    let local = fixture
        .get(&format!("images/{}", images[0]["id"].as_str().unwrap()))
        .await;
    assert_eq!(local.status(), StatusCode::OK);
    let outside = fixture
        .get(&format!("images/{}", images[1]["id"].as_str().unwrap()))
        .await;
    assert_eq!(outside.status(), StatusCode::UNPROCESSABLE_ENTITY);
    let body = axum::body::to_bytes(outside.into_body(), 4096)
        .await
        .unwrap();
    let body: Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(body["error"]["code"], "history-image-unavailable");
    assert_eq!(body["error"]["reason"], "outside-allowed-roots");
    assert!(
        body["error"]["message"]
            .as_str()
            .unwrap()
            .contains("working directory")
    );
}
