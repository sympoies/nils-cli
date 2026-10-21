//! A prompt sent while a Claude turn runs is a `queued_command` attachment. With
//! an image it carries a list of content blocks instead of a string.
use super::*;
use pretty_assertions::assert_eq;

const PNG_BASE64: &str = "iVBORw0KGgo=";

fn queued_row(attachment_fields: Value) -> String {
    let mut attachment = serde_json::json!({
        "type": "queued_command",
        "commandMode": "prompt",
        "origin": {"kind": "human"},
    });
    attachment
        .as_object_mut()
        .unwrap()
        .extend(attachment_fields.as_object().unwrap().clone());
    serde_json::json!({
        "sessionId": "claude-id",
        "type": "attachment",
        "timestamp": "2026-10-01T00:00:02Z",
        "attachment": attachment,
    })
    .to_string()
}

fn text_block(text: &str) -> Value {
    serde_json::json!({"type": "text", "text": text})
}

fn image_block() -> Value {
    serde_json::json!({
        "type": "image",
        "source": {"type": "base64", "media_type": "image/png", "data": PNG_BASE64},
    })
}

fn normalized(line: &str) -> Option<(String, String, Option<String>, bool)> {
    let value: Value = serde_json::from_str(line).unwrap();
    normalize_message("claude", "claude-id", line.as_bytes(), &value)
}

fn image_count(line: &str) -> usize {
    image_sources(&serde_json::from_str::<Value>(line).unwrap()).len()
}

#[test]
fn list_form_queued_prompt_yields_joined_text_and_its_images() {
    let line = queued_row(
        serde_json::json!({"prompt": [text_block("[Image #1]"), text_block("see this"), image_block()]}),
    );
    let (role, text, _, human_prompt) = normalized(&line).expect("queued row is shown");
    assert_eq!(role, "user");
    assert_eq!(text, "[Image #1]\nsee this");
    assert!(human_prompt);
    assert_eq!(image_count(&line), 1);
    assert_eq!(
        parse_history_user_prompt("claude", "claude-id", &line).map(|prompt| prompt.text),
        Some("[Image #1]\nsee this".to_string())
    );
}

#[test]
fn image_only_list_form_queued_prompt_is_kept_for_its_image() {
    let line = queued_row(serde_json::json!({"prompt": [image_block()]}));
    let (role, text, _, _) = normalized(&line).expect("image-only queued row is shown");
    assert_eq!(role, "user");
    assert_eq!(text, "");
    assert_eq!(image_count(&line), 1);
}

#[test]
fn string_queued_prompt_is_unchanged() {
    let line = queued_row(serde_json::json!({"prompt": "queued human"}));
    let (_, text, _, human_prompt) = normalized(&line).expect("string queued row is shown");
    assert_eq!(text, "queued human");
    assert!(human_prompt);
    assert_eq!(image_count(&line), 0);

    let markdown = queued_row(serde_json::json!({"prompt": "![x](data:image/png;base64,AAAA)"}));
    assert_eq!(image_count(&markdown), 0);
}

#[test]
fn non_human_list_form_queued_prompts_stay_hidden() {
    let peer = queued_row(serde_json::json!({
        "origin": {"kind": "peer"},
        "prompt": [text_block("peer message"), image_block()],
    }));
    assert_eq!(normalized(&peer), None);
    assert_eq!(image_count(&peer), 0);

    let machine = queued_row(serde_json::json!({
        "prompt": [text_block("<task-notification>\n<task-id>t</task-id>"), image_block()],
    }));
    assert_eq!(
        parse_history_user_prompt("claude", "claude-id", &machine),
        None
    );
}
