use super::*;
use pretty_assertions::assert_eq;

#[test]
fn history_image_messages_preserve_text_and_image_only_rows() {
    let temp = tempfile::TempDir::new().unwrap();
    let root = temp.path().join("history");
    fs::create_dir(&root).unwrap();
    let meta = serde_json::json!({"type":"session_meta","payload":{"id":"image-regression","cwd":temp.path(),"timestamp":"2026-10-16T00:00:00Z","source":"cli"}});
    let row = |text: Option<&str>| {
        let mut content = vec![
            serde_json::json!({"type":"input_image","image_url":"data:image/png;base64,iVBORw0KGgo="}),
        ];
        if let Some(text) = text {
            content.insert(0, serde_json::json!({"type":"input_text","text":text}));
        }
        serde_json::json!({"type":"response_item","payload":{"type":"message","role":"user","content":content,"internal_chat_message_metadata_passthrough":{"content_item_kinds":["user.text","user.image"]}}})
    };
    fs::write(
        root.join("rollout.jsonl"),
        format!("{meta}\n{}\n{}\n", row(Some("image prompt")), row(None)),
    )
    .unwrap();
    let catalog = HistoryCatalog::new(
        vec![HistorySource {
            provider: "codex".into(),
            agent_profile: None,
            root,
        }],
        archive_root(temp.path()),
        star_root(temp.path()),
    );
    let id = stable_history_id("codex", None, "image-regression");
    for direction in [
        HistoryMessageDirection::Forward,
        HistoryMessageDirection::Latest,
    ] {
        let page = catalog.messages(&id, None, 50, direction).unwrap();
        let json = serde_json::to_value(&page).unwrap();
        assert_eq!(json["messages"][0]["text"], "image prompt");
        assert_eq!(
            json["messages"][0]["images"].as_array().map(Vec::len),
            Some(1)
        );
        assert_eq!(json["messages"].as_array().unwrap().len(), 2);
        assert_eq!(json["messages"][1]["text"], "");
        assert_eq!(
            json["messages"][1]["images"].as_array().map(Vec::len),
            Some(1)
        );
    }
}

#[test]
fn claude_history_images_keep_paging_and_resolve_nested_embedded_sources() {
    let temp = tempfile::TempDir::new().unwrap();
    let root = temp.path().join("projects");
    fs::create_dir_all(root.join("repo")).unwrap();
    let image = serde_json::json!({"type":"image","source":{"type":"base64","media_type":"image/png","data":"iVBORw0KGgo="}});
    let row = |content: Value| serde_json::json!({"sessionId":"claude-image","cwd":temp.path(),"type":"user","promptSource":"typed","message":{"role":"user","content":content}});
    let first = row(serde_json::json!([{"type":"text","text":"first"}, image.clone()]));
    let second = row(serde_json::json!([{"type":"tool_result","content":[image.clone()]}]));
    let third = row(serde_json::json!([{"type":"text","text":"last"}, image]));
    fs::write(
        root.join("repo/claude-image.jsonl"),
        format!("{first}\n{second}\n{third}\n"),
    )
    .unwrap();
    let catalog = HistoryCatalog::new(
        vec![HistorySource {
            provider: "claude".into(),
            agent_profile: None,
            root,
        }],
        archive_root(temp.path()),
        star_root(temp.path()),
    );
    let id = stable_history_id("claude", None, "claude-image");
    let latest = catalog
        .messages(&id, None, 2, HistoryMessageDirection::Latest)
        .unwrap();
    assert_eq!(latest.messages.len(), 2);
    assert_eq!(latest.messages[0].text, "");
    assert!(!latest.messages[0].human_prompt);
    assert_eq!(latest.messages[1].text, "last");
    let older = catalog
        .messages(
            &id,
            latest.older_cursor.as_deref(),
            2,
            HistoryMessageDirection::Older,
        )
        .unwrap();
    assert_eq!(older.messages.len(), 1);
    assert_eq!(older.messages[0].text, "first");
    for message in older.messages.iter().chain(&latest.messages) {
        assert_eq!(message.images.len(), 1);
        assert!(message.images[0].id.len() <= 96);
        let image = catalog.image(&id, &message.images[0].id).unwrap();
        assert_eq!(image.mime, "image/png");
        assert_eq!(image.bytes, b"\x89PNG\r\n\x1a\n");
        assert!(matches!(
            catalog.image("unrelated-history", &message.images[0].id),
            Err(history_images::ImageReadError::Deleted)
        ));
    }
}
