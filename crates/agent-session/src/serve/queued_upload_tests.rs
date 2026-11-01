//! Generic Claude Code 2.1.316 carrier fixtures from attributed structural
//! evidence; these tests do not launch Claude or claim native/client acceptance.
use super::*;
use pretty_assertions::{assert_eq, assert_ne};

const JPEG: &str = "/9j/4AAQSkZJRgABAQAAAQABAAD/2wBDAAgGBgcGBQgHBwcJCQgKDBQNDAsLDBkSEw8UHRofHh0aHBwgJC4nICIsIxwcKDcpLDAxNDQ0Hyc5PTgyPC4zNDL/2wBDAQkJCQwLDBgNDRgyIRwhMjIyMjIyMjIyMjIyMjIyMjIyMjIyMjIyMjIyMjIyMjIyMjIyMjIyMjIyMjIyMjIyMjL/wAARCAACAAIDASIAAhEBAxEB/8QAHwAAAQUBAQEBAQEAAAAAAAAAAAECAwQFBgcICQoL/8QAtRAAAgEDAwIEAwUFBAQAAAF9AQIDAAQRBRIhMUEGE1FhByJxFDKBkaEII0KxwRVS0fAkM2JyggkKFhcYGRolJicoKSo0NTY3ODk6Q0RFRkdISUpTVFVWV1hZWmNkZWZnaGlqc3R1dnd4eXqDhIWGh4iJipKTlJWWl5iZmqKjpKWmp6ipqrKztLW2t7i5usLDxMXGx8jJytLT1NXW19jZ2uHi4+Tl5ufo6erx8vP09fb3+Pn6/8QAHwEAAwEBAQEBAQEBAQAAAAAAAAECAwQFBgcICQoL/8QAtREAAgECBAQDBAcFBAQAAQJ3AAECAxEEBSExBhJBUQdhcRMiMoEIFEKRobHBCSMzUvAVYnLRChYkNOEl8RcYGRomJygpKjU2Nzg5OkNERUZHSElKU1RVVldYWVpjZGVmZ2hpanN0dXZ3eHl6goOEhYaHiImKkpOUlZaXmJmaoqOkpaanqKmqsrO0tba3uLm6wsPExcbHyMnK0tPU1dbX2Nna4uPk5ebn6Onq8vP09fb3+Pn6/9oADAMBAAIRAxEAPwDGooor6Y+XP//Z";

fn row(fixture: &Fixture, row: Value) {
    use std::io::Write;
    writeln!(
        fs::OpenOptions::new()
            .append(true)
            .open(&fixture.transcript)
            .unwrap(),
        "{row}"
    )
    .unwrap();
}

fn queued(fixture: &Fixture, prompt: Value) {
    row(
        fixture,
        json!({"type":"attachment", "sessionId":"own-provider",
        "cwd":fixture._temp.path(), "version":"2.1.316", "uuid":"synthetic-queued",
        "isSidechain":false, "rendered":[{"content":prompt}],
        "attachment":{"type":"queued_command", "prompt":prompt,
            "source_uuid":"synthetic-source", "delivery_id":"synthetic-delivery",
            "commandMode":"prompt", "origin":{"kind":"human"}, "humanTurn":true}}),
    );
}

async fn ordered_string_upload(path_first: bool) {
    let fixture = Fixture::for_provider("claude");
    let jpeg = base64::engine::general_purpose::STANDARD
        .decode(JPEG)
        .unwrap();
    let upload = fixture.upload("own", "synthetic.jpg", &jpeg).await;
    let prompt = if path_first {
        format!("'{}'\nSynthetic caption", upload.display())
    } else {
        format!("Synthetic caption\n'{}'", upload.display())
    };
    // The attributed native sample was queued path-first and idle caption-first;
    // the other ordering is a generic string variant, not a raw recording.
    row(
        &fixture,
        json!({"type":"user", "sessionId":"own-provider",
        "cwd":fixture._temp.path(), "version":"2.1.316", "uuid":"synthetic-idle",
        "promptSource":"typed", "turnOrigin":"human", "isSidechain":false,
        "message":{"role":"user", "content":prompt}}),
    );
    queued(&fixture, json!(prompt));
    let original = fs::read(&fixture.transcript).unwrap();
    let page = fixture.refresh().await;
    let messages = page["data"]["messages"].as_array().unwrap();
    assert_eq!(messages.len(), 2);
    for (index, message) in messages.iter().enumerate() {
        assert_eq!(message["role"], "user");
        assert_eq!(
            message["images"].as_array().map(Vec::len),
            Some(1),
            "idle/queued carrier index {index} must project one owned upload"
        );
        assert_eq!(message["text"], "Synthetic caption");
        let image = &message["images"][0];
        assert_eq!(image["origin"], "user_attachment");
        let reference = image["attachment_ref"].as_str().unwrap();
        assert!(reference.starts_with("upload-"));
        assert!((1..=128).contains(&reference.len()));
        assert!(!reference.contains('/'));
        let parts = message["parts"].as_array().unwrap();
        let expected = if path_first {
            ["image", "text"]
        } else {
            ["text", "image"]
        };
        assert_eq!(
            parts
                .iter()
                .map(|p| p["kind"].as_str().unwrap())
                .collect::<Vec<_>>(),
            expected
        );
        let part = &parts[usize::from(!path_first)];
        assert_eq!(part["ref"], image["id"]);
        assert_eq!(part["origin"], image["origin"]);
        assert_eq!(part["attachment_ref"], image["attachment_ref"]);
        assert_eq!(parts[usize::from(path_first)]["text"], "Synthetic caption");
        let id = image["id"].as_str().unwrap();
        let denied = super::super::super::router(fixture.state.clone())
            .oneshot(
                Request::get(format!("/history/sessions/{}/images/{id}", fixture.history))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(denied.status(), StatusCode::UNAUTHORIZED);
        let response = fixture.image(id).await;
        assert_eq!(response.status(), StatusCode::OK);
        assert_eq!(response.headers()[CONTENT_TYPE], "image/jpeg");
        assert_eq!(
            axum::body::to_bytes(response.into_body(), 4096)
                .await
                .unwrap()
                .as_ref(),
            jpeg
        );
    }
    assert_eq!(
        messages[0]["images"][0]["attachment_ref"],
        messages[1]["images"][0]["attachment_ref"]
    );
    assert_ne!(
        messages[0]["images"][0]["id"],
        messages[1]["images"][0]["id"]
    );
    let first = fixture
        .state
        .history_catalog
        .messages(&fixture.history, None, 1, HistoryMessageDirection::Forward)
        .unwrap();
    let second = fixture
        .state
        .history_catalog
        .messages(
            &fixture.history,
            first.next_cursor.as_deref(),
            1,
            HistoryMessageDirection::Forward,
        )
        .unwrap();
    assert_eq!(first.messages.len(), 1);
    assert_eq!(second.messages.len(), 1);
    assert_eq!(
        serde_json::to_value(&second.messages[0].images).unwrap(),
        messages[1]["images"]
    );
    assert_eq!(second.messages[0].text, "Synthetic caption");
    assert_eq!(fixture.refresh().await, page);
    assert_eq!(fs::read(&fixture.transcript).unwrap(), original);
}

#[tokio::test]
async fn queued_string_path_before_caption_projects_owned_jpeg() {
    ordered_string_upload(true).await;
}

#[tokio::test]
async fn queued_string_caption_before_path_projects_owned_jpeg() {
    ordered_string_upload(false).await;
}

#[tokio::test]
async fn queued_upload_paths_keep_managed_raster_boundary() {
    let fixture = Fixture::for_provider("claude");
    let own = fixture.upload("own", "valid.png", &fixture.png).await;
    let other = fixture.upload("other", "other.png", &fixture.png).await;
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
    let prompt = refused
        .iter()
        .map(|p| format!("'{}'", p.display()))
        .collect::<Vec<_>>()
        .join("\n");
    queued(&fixture, json!(prompt));
    let page = fixture.refresh().await;
    let message = &page["data"]["messages"][0];
    assert!(message.get("images").is_none());
    assert_eq!(message["text"], prompt);
    assert_eq!(message["parts"].as_array().unwrap().len(), 1);
}

#[tokio::test]
async fn queued_structured_images_keep_parts_order_and_image_only_turns() {
    let fixture = Fixture::for_provider("claude");
    let upload = fixture.upload("own", "structured.png", &fixture.png).await;
    let image = json!({"type":"image", "source":{"type":"file", "path":upload}});
    queued(
        &fixture,
        json!([image, {"type":"text", "text":"between"}, image]),
    );
    queued(&fixture, json!([image]));
    let original = fs::read(&fixture.transcript).unwrap();
    let page = fixture.refresh().await;
    let messages = page["data"]["messages"].as_array().unwrap();
    assert_eq!(messages.len(), 2);
    assert_eq!(messages[0]["images"].as_array().unwrap().len(), 2);
    assert_eq!(
        messages[0]["parts"]
            .as_array()
            .unwrap()
            .iter()
            .map(|p| p["kind"].as_str().unwrap())
            .collect::<Vec<_>>(),
        ["image", "text", "image"]
    );
    assert_eq!(messages[0]["parts"][1]["text"], "between");
    assert_eq!(messages[1]["text"], "");
    assert_eq!(messages[1]["images"].as_array().unwrap().len(), 1);
    assert_eq!(messages[1]["parts"][0]["kind"], "image");
    for message in messages {
        let parts = message["parts"]
            .as_array()
            .unwrap()
            .iter()
            .filter(|p| p["kind"] == "image");
        for (image, part) in message["images"].as_array().unwrap().iter().zip(parts) {
            assert_eq!(image["origin"], "user_attachment");
            assert!(image["attachment_ref"].is_string());
            assert_eq!(part["ref"], image["id"]);
            assert_eq!(part["origin"], image["origin"]);
            assert_eq!(part["attachment_ref"], image["attachment_ref"]);
            assert_eq!(
                fixture.image(image["id"].as_str().unwrap()).await.status(),
                StatusCode::OK
            );
        }
    }
    assert_eq!(fs::read(&fixture.transcript).unwrap(), original);
}
