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

#[test]
fn image_text_incremental_ids_match_pre_attachment_applied_memory() {
    let temp = tempfile::TempDir::new().unwrap();
    let root = temp.path().join("history");
    fs::create_dir(&root).unwrap();
    let meta = serde_json::json!({"type":"session_meta","payload":{"id":"stable-image-id","cwd":temp.path(),"timestamp":"2026-10-16T00:00:00Z","source":"cli"}});
    let row = serde_json::json!({"type":"response_item","payload":{"type":"message","role":"user","content":[{"type":"input_text","text":"Inspect the image"},{"type":"input_image","image_url":"data:image/png;base64,iVBORw0KGgo="}],"internal_chat_message_metadata_passthrough":{"turn_id":"stable-turn","content_item_kinds":["user.text","user.image"]}}});
    fs::write(root.join("rollout.jsonl"), format!("{meta}\n{row}\n")).unwrap();
    let catalog = HistoryCatalog::new(
        vec![HistorySource {
            provider: "codex".into(),
            agent_profile: None,
            root,
        }],
        archive_root(temp.path()),
        star_root(temp.path()),
    );
    let id = stable_history_id("codex", None, "stable-image-id");
    let snapshot = catalog.snapshot();
    let session = &snapshot.incremental_segments[&id][0];
    let old_id = incremental_message_id(session, &row, "user", "Inspect the image", None, true);
    let replay = catalog
        .incremental_messages_with_applied_ids(&id, None, &[old_id], 1024 * 1024)
        .unwrap();
    assert_eq!(
        replay.messages.len(),
        0,
        "Previously applied text-bearing images must remain deduplicated after upgrade/recovery"
    );
}

#[test]
fn codex_history_links_the_caller_request_before_receipt_lookup() {
    let temp = tempfile::TempDir::new().unwrap();
    let history_root = temp.path().join("history");
    let state_dir = temp.path().join("state");
    fs::create_dir_all(&history_root).unwrap();
    fs::create_dir_all(state_dir.join("sessions/managed-session")).unwrap();
    fs::create_dir_all(state_dir.join("sessions/other-managed-session")).unwrap();
    let meta = serde_json::json!({"type":"session_meta","payload":{"id":"binding-thread","cwd":temp.path(),"timestamp":"2026-10-16T00:00:00Z","source":"cli"}});
    let row = serde_json::json!({"type":"response_item","payload":{"type":"message","role":"user","content":[{"type":"input_text","text":"continue"}],"internal_chat_message_metadata_passthrough":{"turn_id":"native-turn-1","content_item_kinds":["user.text"]}}});
    fs::write(
        history_root.join("rollout.jsonl"),
        format!("{meta}\n{row}\n"),
    )
    .unwrap();
    let stored_result = serde_json::json!({
        "receipt": {
            "request_id": "attempt-1",
            "session_id": "managed-session",
            "session_incarnation": "launch-a",
            "outcome": "submitted",
            "code": null
        },
        "status": 200,
        "queued": false,
        "history_turn_id": "native-turn-1",
        "history_session_id": "binding-thread"
    });
    fs::write(
        state_dir.join("sessions/managed-session/prompt-receipt-key.result"),
        serde_json::to_vec(&stored_result).unwrap(),
    )
    .unwrap();
    let other_thread_result = serde_json::json!({
        "receipt": {
            "request_id": "other-attempt",
            "session_id": "other-managed-session",
            "session_incarnation": "launch-other",
            "outcome": "submitted",
            "code": null
        },
        "status": 200,
        "queued": false,
        "history_turn_id": "native-turn-1",
        "history_session_id": "other-thread"
    });
    fs::write(
        state_dir.join("sessions/other-managed-session/prompt-receipt-key.result"),
        serde_json::to_vec(&other_thread_result).unwrap(),
    )
    .unwrap();
    let catalog = HistoryCatalog::new(
        vec![HistorySource {
            provider: "codex".into(),
            agent_profile: None,
            root: history_root,
        }],
        archive_root(temp.path()),
        star_root(temp.path()),
    )
    .with_session_state_dir(state_dir);
    let history_id = stable_history_id("codex", None, "binding-thread");

    for direction in [
        HistoryMessageDirection::Forward,
        HistoryMessageDirection::Latest,
    ] {
        let page = catalog.messages(&history_id, None, 10, direction).unwrap();
        let json = serde_json::to_value(page).unwrap();
        assert_eq!(json["messages"][0]["text"], "continue");
        assert_eq!(
            json["messages"][0]["prompt_request"]["request_id"],
            "attempt-1"
        );
        assert_eq!(
            json["messages"][0]["prompt_request"]["session_id"],
            "managed-session"
        );
        assert_eq!(
            json["messages"][0]["prompt_request"]["session_incarnation"],
            "launch-a"
        );
        assert!(!json.to_string().contains("native-turn-1"));
        assert!(!json.to_string().contains("binding-thread"));
    }
}
