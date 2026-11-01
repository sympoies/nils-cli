use super::*;

#[test]
fn codex_history_turn_ids_distinguish_duplicate_text_and_image_only_turns() {
    let session = history_session("turn-binding", "2026-09-09T00:00:00Z");
    let rows = [
        serde_json::json!({"timestamp":"2026-09-09T00:00:00Z","type":"response_item","payload":{"type":"message","role":"user","content":[{"type":"input_text","text":"continue"}],"internal_chat_message_metadata_passthrough":{"turn_id":"turn-one","content_item_kinds":["user.text"]}}}),
        serde_json::json!({"timestamp":"2026-09-09T00:00:01Z","type":"response_item","payload":{"type":"message","role":"user","content":[{"type":"input_text","text":"continue"}],"internal_chat_message_metadata_passthrough":{"turn_id":"turn-two","content_item_kinds":["user.text"]}}}),
        serde_json::json!({"timestamp":"2026-09-09T00:00:02Z","type":"response_item","payload":{"type":"message","role":"user","content":[{"type":"input_image","image_url":"data:image/png;base64,iVBORw0KGgo="}],"internal_chat_message_metadata_passthrough":{"turn_id":"turn-image","content_item_kinds":["user.image"]}}}),
    ];
    let bytes = rows
        .iter()
        .map(Value::to_string)
        .collect::<Vec<_>>()
        .join("\n")
        + "\n";
    let mut reader = Cursor::new(bytes.as_bytes());
    let mut page = read_messages_incremental_from_reader(
        &session,
        &mut reader,
        bytes.len() as u64,
        0,
        64 * 1024,
        false,
    )
    .unwrap();

    assert_eq!(page.messages.len(), 3);
    assert_eq!(page.messages[0].text, "continue");
    assert_eq!(page.messages[1].text, "continue");
    assert_eq!(
        page.messages[0].history_turn_id.as_deref(),
        Some("turn-one")
    );
    assert_eq!(
        page.messages[1].history_turn_id.as_deref(),
        Some("turn-two")
    );
    assert_eq!(page.messages[2].text, "");
    assert_eq!(
        page.messages[2].history_turn_id.as_deref(),
        Some("turn-image")
    );

    let bindings = HashMap::from([
        (
            "turn-one".to_string(),
            PromptRequestBinding {
                request_id: "attempt-one".into(),
                session_id: "session-one".into(),
                session_incarnation: "launch-one".into(),
            },
        ),
        (
            "turn-two".to_string(),
            PromptRequestBinding {
                request_id: "attempt-two".into(),
                session_id: "session-one".into(),
                session_incarnation: "launch-one".into(),
            },
        ),
        (
            "turn-image".to_string(),
            PromptRequestBinding {
                request_id: "attempt-image".into(),
                session_id: "session-one".into(),
                session_incarnation: "launch-one".into(),
            },
        ),
    ]);
    history_images::messages::apply_prompt_request_bindings(&mut page.messages, &bindings);
    assert_eq!(
        page.messages[0].prompt_request.as_ref().unwrap().request_id,
        "attempt-one"
    );
    assert_eq!(
        page.messages[1].prompt_request.as_ref().unwrap().request_id,
        "attempt-two"
    );
    assert_eq!(
        page.messages[2].prompt_request.as_ref().unwrap().request_id,
        "attempt-image"
    );
    assert_eq!(
        serde_json::to_value(&page.messages[0]).unwrap()["prompt_request"]["request_id"],
        "attempt-one"
    );

    let reverse = history_images::read_messages_reverse_from_reader(
        &session,
        &mut reader,
        bytes.len() as u64,
        None,
        3,
        Instant::now() + Duration::from_secs(1),
        false,
        &history_images::native_notices::Bindings::default(),
    )
    .unwrap();
    assert_eq!(
        reverse
            .messages
            .iter()
            .map(|message| message.history_turn_id.as_deref())
            .collect::<Vec<_>>(),
        [Some("turn-one"), Some("turn-two"), Some("turn-image")]
    );
}
