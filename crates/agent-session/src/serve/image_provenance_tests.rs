//! Synthetic rows exercise the authenticated upload/history/read boundary.
use super::*;
use pretty_assertions::{assert_eq, assert_ne};
use std::io::Write;

impl Fixture {
    fn row(&self, row: Value) {
        writeln!(
            fs::OpenOptions::new()
                .append(true)
                .open(&self.transcript)
                .unwrap(),
            "{row}"
        )
        .unwrap();
    }
    fn claude_row(&self, content: Value, role: &str) {
        self.row(
            json!({"type":role,"sessionId":"own-provider","cwd":self._temp.path(),
            "uuid":"fixture-row","version":"2.1.312","isSidechain":false,"isMeta":null,
            "message":{"role":role,"content":content}}),
        );
    }
}

fn assert_metadata(message: &Value, origins: &[Option<&str>]) {
    let images = message["images"].as_array().unwrap();
    assert_eq!(images.len(), origins.len());
    let parts: Vec<_> = message["parts"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|p| p["kind"] == "image")
        .collect();
    assert_eq!(parts.len(), images.len());
    for ((image, part), origin) in images.iter().zip(parts).zip(origins) {
        assert_eq!(image["origin"].as_str(), *origin);
        assert_eq!(part["origin"], image["origin"]);
        assert_eq!(part["attachment_ref"], image["attachment_ref"]);
        assert_eq!(part["ref"], image["id"]);
        if origin.is_none() {
            assert!(image.get("origin").is_none());
            assert!(part.get("origin").is_none());
        }
        if let Some(reference) = image.get("attachment_ref") {
            let reference = reference.as_str().unwrap();
            assert!((1..=128).contains(&reference.len()));
            assert!(
                reference
                    .bytes()
                    .all(|b| b.is_ascii_alphanumeric() || b"._~-".contains(&b))
            );
        } else {
            assert!(part.get("attachment_ref").is_none());
        }
    }
}

#[tokio::test]
async fn claude_quoted_owned_upload_caption_projects_without_rewriting_transcript() {
    let fixture = Fixture::for_provider("claude");
    let upload = fixture.upload("own", "sample.png", &fixture.png).await;
    // Claude Code 2.1.312: one user string, quoted upload then newline/caption;
    // no structured image block, source metadata, or promptSource.
    fixture.claude_row(
        json!(format!("'{}'\nDescribe this image.", upload.display())),
        "user",
    );
    let before = fs::read(&fixture.transcript).unwrap();
    let page = fixture.refresh().await;
    let message = &page["data"]["messages"][0];
    assert_eq!(message["images"].as_array().map(Vec::len), Some(1));
    assert_eq!(message["role"], "user");
    assert_eq!(message["text"], "Describe this image.");
    assert_eq!(message["parts"][0]["kind"], "image");
    assert_eq!(message["parts"][1]["text"], "Describe this image.");
    assert_eq!(
        fixture
            .image(message["images"][0]["id"].as_str().unwrap())
            .await
            .status(),
        StatusCode::OK
    );
    assert_eq!(fs::read(&fixture.transcript).unwrap(), before);
}

#[tokio::test]
async fn owned_upload_metadata_correlates_user_tool_and_paged_reads_for_both_providers() {
    for provider in ["codex", "claude"] {
        let fixture = Fixture::for_provider(provider);
        let first = fixture.upload("own", "same.png", &fixture.png).await;
        let second = fixture.upload("own", "same.png", &fixture.png).await;
        assert_ne!(first, second);
        let reread = first
            .parent()
            .unwrap()
            .join(".")
            .join(first.file_name().unwrap());
        if provider == "codex" {
            fixture.append(json!([{"type":"input_text","text":format!("'{}'\ncaption", first.display())}, {"type":"input_image","path":second}]));
            fixture.row(json!({"type":"response_item","payload":{"type":"function_call_output","call_id":"read-1","output":[
                {"type":"input_image","path":reread},{"type":"input_image","image_url":format!("data:image/png;base64,{PNG}")}]}}));
        } else {
            fixture.claude_row(json!([{"type":"text","text":format!("'{}'\ncaption", first.display())},{"type":"image","source":{"type":"file","path":second}}]), "user");
            fixture.claude_row(json!([{"type":"tool_result","tool_use_id":"read-1","content":[
                {"type":"image","source":{"type":"file","path":reread}}, {"type":"image","source":{"type":"base64","media_type":"image/png","data":PNG}}]}]), "user");
        }
        let original = fs::read(&fixture.transcript).unwrap();
        let page = fixture.refresh().await;
        let messages = page["data"]["messages"].as_array().unwrap();
        assert_eq!(messages.len(), 2);
        let reference = messages[0]["images"][0]["attachment_ref"]
            .as_str()
            .expect("owned user reference");
        assert_eq!(messages[1]["images"][0]["attachment_ref"], reference);
        assert_ne!(messages[0]["images"][1]["attachment_ref"], reference);
        assert!(messages[0]["images"][1]["attachment_ref"].is_string());
        assert!(messages[1]["images"][1].get("attachment_ref").is_none());
        assert_ne!(
            messages[0]["images"][0]["id"],
            messages[1]["images"][0]["id"]
        );
        assert_metadata(
            &messages[0],
            &[Some("user_attachment"), Some("user_attachment")],
        );
        assert_metadata(&messages[1], &[Some("tool_result"), Some("tool_result")]);
        for message in messages {
            for image in message["images"].as_array().unwrap() {
                assert_eq!(
                    fixture.image(image["id"].as_str().unwrap()).await.status(),
                    StatusCode::OK
                );
            }
        }
        assert_eq!(fixture.refresh().await, page);
        let first_page = fixture
            .state
            .history_catalog
            .messages(&fixture.history, None, 1, HistoryMessageDirection::Forward)
            .unwrap();
        let second_page = fixture
            .state
            .history_catalog
            .messages(
                &fixture.history,
                first_page.next_cursor.as_deref(),
                1,
                HistoryMessageDirection::Forward,
            )
            .unwrap();
        assert_eq!(
            serde_json::to_value(&first_page.messages[0].images).unwrap(),
            messages[0]["images"]
        );
        assert_eq!(
            serde_json::to_value(&second_page.messages[0].images).unwrap(),
            messages[1]["images"]
        );
        assert_eq!(fs::read(&fixture.transcript).unwrap(), original);
    }
}

#[tokio::test]
async fn mixed_claude_images_use_each_block_provenance_and_unknown_origin_is_absent() {
    let fixture = Fixture::for_provider("claude");
    let upload = fixture.upload("own", "mixed.png", &fixture.png).await;
    let image = json!({"type":"image","source":{"type":"file","path":upload}});
    fixture.claude_row(json!([{"type":"text","text":"caption"}, image, {"type":"tool_result","tool_use_id":"read","content":[image]}]), "user");
    fixture.claude_row(json!([image]), "assistant");
    let page = fixture.refresh().await;
    let mixed = &page["data"]["messages"][0];
    assert_metadata(mixed, &[Some("user_attachment"), Some("tool_result")]);
    assert_eq!(
        mixed["images"][0]["attachment_ref"],
        mixed["images"][1]["attachment_ref"]
    );
    assert_metadata(&page["data"]["messages"][1], &[None]);
}

#[tokio::test]
async fn refused_upload_paths_do_not_gain_correlation_or_image_read_authority() {
    for provider in ["codex", "claude"] {
        let fixture = Fixture::for_provider(provider);
        let own = fixture.upload("own", "valid.png", &fixture.png).await;
        let other = fixture.upload("other", "same.png", &fixture.png).await;
        let text = fixture.upload("own", "notes.txt", b"caption").await;
        let fake = fixture.upload("own", "fake.png", b"not raster").await;
        let outside = fixture._temp.path().join("outside.png");
        fs::write(&outside, &fixture.png).unwrap();
        let linked = own.parent().unwrap().join("linked.png");
        std::os::unix::fs::symlink(&outside, &linked).unwrap();
        let traversal = own
            .parent()
            .unwrap()
            .join("../attachments")
            .join(own.file_name().unwrap());
        let refused = [other, text, fake, outside, linked, traversal];
        let quoted = refused
            .iter()
            .map(|p| format!("'{}'", p.display()))
            .collect::<Vec<_>>()
            .join("\n");
        if provider == "claude" {
            fixture.claude_row(json!(quoted), "user");
        } else {
            fixture.append(json!([{"type":"input_text","text":quoted}]));
        }
        let blocks: Vec<_> = refused
            .iter()
            .map(|p| json!({"type":"image","source":{"type":"file","path":p}}))
            .collect();
        if provider == "claude" {
            fixture.claude_row(
                json!([{"type":"tool_result","tool_use_id":"read","content":blocks}]),
                "user",
            );
        } else {
            fixture.row(json!({"type":"response_item","payload":{"type":"mcp_tool_result","output":blocks}}));
        }
        let page = fixture.refresh().await;
        let messages = page["data"]["messages"].as_array().unwrap();
        assert!(messages[0].get("images").is_none());
        assert_eq!(messages[0]["text"], quoted);
        assert_metadata(&messages[1], &[Some("tool_result"); 6]);
        for image in messages[1]["images"].as_array().unwrap() {
            assert!(image.get("attachment_ref").is_none());
        }
        // This fixture records a broad cwd: regular files beneath it were
        // already readable, including unowned uploads. They gain no reference.
        for index in [0, 3] {
            assert_eq!(
                fixture
                    .image(messages[1]["images"][index]["id"].as_str().unwrap())
                    .await
                    .status(),
                StatusCode::OK
            );
        }
        for index in [1, 2, 4, 5] {
            assert_ne!(
                fixture
                    .image(messages[1]["images"][index]["id"].as_str().unwrap())
                    .await
                    .status(),
                StatusCode::OK
            );
        }
    }
}

#[tokio::test]
async fn claude_projection_preserves_structured_markdown_and_image_only_order() {
    let fixture = Fixture::for_provider("claude");
    let upload = fixture.upload("own", "image.png", &fixture.png).await;
    fixture.claude_row(json!([
        {"type":"image","source":{"type":"base64","media_type":"image/png","data":PNG}},
        {"type":"text","text":format!("![before](data:image/png;base64,{PNG}) '{}'\nafter", upload.display())},
        {"type":"image","source":{"type":"base64","media_type":"image/png","data":PNG}}
    ]), "user");
    fixture.claude_row(json!(format!("'{}'", upload.display())), "user");
    let page = fixture.refresh().await;
    let messages = page["data"]["messages"].as_array().unwrap();
    assert_eq!(messages.len(), 2);
    assert_metadata(&messages[0], &[Some("user_attachment"); 4]);
    let parts = messages[0]["parts"].as_array().unwrap();
    assert_eq!(
        parts
            .iter()
            .map(|p| p["kind"].as_str().unwrap())
            .collect::<Vec<_>>(),
        ["image", "text", "image", "image", "text", "image"]
    );
    assert_eq!(parts[4]["text"], "after");
    assert_eq!(messages[1]["text"], "");
    assert_metadata(&messages[1], &[Some("user_attachment")]);
    assert_eq!(messages[1]["parts"][0]["kind"], "image");
    assert_eq!(
        messages[0]["images"][2]["attachment_ref"],
        messages[1]["images"][0]["attachment_ref"]
    );
    for message in messages {
        for image in message["images"].as_array().unwrap() {
            assert_eq!(
                fixture.image(image["id"].as_str().unwrap()).await.status(),
                StatusCode::OK
            );
        }
    }
}

#[tokio::test]
async fn codex_tool_output_forms_keep_origin_without_inventing_upload_identity() {
    let fixture = Fixture::new();
    for kind in [
        "function_call_output",
        "custom_tool_call_output",
        "mcp_tool_result",
    ] {
        fixture.row(
            json!({"type":"response_item","payload":{"type":kind,"call_id":"read",
            "output":[{"type":"input_image","image_url":format!("data:image/png;base64,{PNG}")}]}}),
        );
    }
    fixture.row(
        json!({"type":"response_item","payload":{"type":"message","role":"assistant",
        "content":[{"type":"output_image","image_url":format!("data:image/png;base64,{PNG}")}]}}),
    );
    let page = fixture.refresh().await;
    let messages = page["data"]["messages"].as_array().unwrap();
    assert_eq!(messages.len(), 4);
    for message in &messages[..3] {
        assert_metadata(message, &[Some("tool_result")]);
        assert!(message["images"][0].get("attachment_ref").is_none());
    }
    assert_metadata(&messages[3], &[None]);
}

#[tokio::test]
async fn correlated_metadata_respects_the_parts_byte_cap() {
    let fixture = Fixture::new();
    let upload = fixture.upload("own", "image.png", &fixture.png).await;
    let mut content = vec![json!({"type":"input_text","text":"x".repeat(61_000)})];
    content.extend((0..16).map(|_| json!({"type":"input_image","path":upload})));
    fixture.append(json!(content));
    let page = fixture.refresh().await;
    let message = &page["data"]["messages"][0];
    assert_eq!(message["images"].as_array().unwrap().len(), 16);
    assert!(message["images"][0]["attachment_ref"].is_string());
    assert!(serde_json::to_vec(&message["parts"]).unwrap().len() <= 64 * 1024);
}
