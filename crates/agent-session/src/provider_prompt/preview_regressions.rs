use super::*;
use pretty_assertions::assert_eq;
use serde_json::json;
use std::io::Write;

fn append(path: &Path, row: Value) {
    let mut file = fs::OpenOptions::new().append(true).open(path).unwrap();
    writeln!(file, "{row}").unwrap();
}

fn user(provider: ProviderKind, text: &str) -> Value {
    match provider {
        ProviderKind::Claude => {
            json!({"type":"user", "sessionId":"synthetic", "promptSource":"typed", "uuid":text,
            "message":{"role":"user", "content":text}})
        }
        ProviderKind::Codex => {
            json!({"type":"event_msg", "payload":{"type":"user_message", "message":text}})
        }
    }
}

#[test]
fn queued_claude_input_updates_live_and_cold_preview() {
    let tmp = tempfile::tempdir().unwrap();
    let path = tmp.path().join("synthetic.jsonl");
    fs::write(&path, "").unwrap();
    append(&path, user(ProviderKind::Claude, "initial request"));
    let mut tracker = ProviderLastPromptTracker::open_path(
        ProviderKind::Claude,
        "synthetic",
        path.clone(),
        Duration::ZERO,
    )
    .unwrap();
    append(
        &path,
        json!({"type":"queue-operation", "operation":"enqueue", "sessionId":"synthetic", "content":"queued request"}),
    );
    let ProviderLastPromptRefresh::Current(Some(prompt)) = tracker.refresh() else {
        panic!("current preview")
    };
    assert_eq!(prompt.text, "queued request");
    assert_eq!(tracker.last_message().unwrap().text, "queued request");
    let mut cold = ProviderLastPromptTracker::open_path(
        ProviderKind::Claude,
        "synthetic",
        path.clone(),
        Duration::ZERO,
    )
    .unwrap();
    let ProviderLastPromptRefresh::Current(Some(prompt)) = cold.refresh() else {
        panic!("cold preview")
    };
    assert_eq!(prompt.text, "queued request");
    assert_eq!(cold.last_message(), tracker.last_message());
    append(
        &path,
        json!({"type":"attachment", "sessionId":"synthetic", "attachment":{
        "type":"queued_command", "commandMode":"prompt", "origin":{"kind":"human"}, "prompt":"another queued request"}}),
    );
    append(
        &path,
        json!({"type":"last-prompt", "sessionId":"synthetic", "lastPrompt":"initial request"}),
    );
    let ProviderLastPromptRefresh::Current(Some(prompt)) = tracker.cached_refresh() else {
        panic!("current attachment preview")
    };
    assert_eq!(prompt.text, "another queued request");
    assert_eq!(
        tracker.last_message().unwrap().text,
        "another queued request"
    );
}

#[test]
fn codex_mid_turn_input_is_current_at_each_growing_list_poll() {
    let tmp = tempfile::tempdir().unwrap();
    let path = tmp.path().join("synthetic.jsonl");
    fs::write(&path, "").unwrap();
    append(&path, user(ProviderKind::Codex, "initial request"));
    let mut tracker = ProviderLastPromptTracker::open_path(
        ProviderKind::Codex,
        "synthetic",
        path.clone(),
        Duration::ZERO,
    )
    .unwrap();
    for text in ["queued request", "later request"] {
        append(
            &path,
            json!({"type":"event_msg", "payload":{"type":"task_started"}}),
        );
        append(&path, user(ProviderKind::Codex, text));
        let ProviderLastPromptRefresh::Current(Some(prompt)) = tracker.cached_refresh() else {
            panic!("warm list poll must consume bounded appends")
        };
        assert_eq!(prompt.text, text);
        assert_eq!(tracker.last_message().unwrap().role, "user");
        assert_eq!(tracker.last_message().unwrap().text, text);
    }
}

#[test]
fn claude_queue_interruption_keeps_latest_accepted_input_until_next_input() {
    let tmp = tempfile::tempdir().unwrap();
    let path = tmp.path().join("synthetic.jsonl");
    fs::write(&path, "").unwrap();
    append(&path, user(ProviderKind::Claude, "initial request"));
    let mut tracker = ProviderLastPromptTracker::open_path(
        ProviderKind::Claude,
        "synthetic",
        path.clone(),
        Duration::from_secs(60),
    )
    .unwrap();
    append(
        &path,
        json!({"type":"queue-operation", "operation":"enqueue", "sessionId":"synthetic", "content":"accepted queued request"}),
    );
    assert!(matches!(
        tracker.cached_refresh(),
        ProviderLastPromptRefresh::Current(_)
    ));
    // A non-enqueue queue update is queue state, not a new human message.
    append(
        &path,
        json!({"type":"queue-operation", "operation":"dequeue", "sessionId":"synthetic", "content":"queue state"}),
    );
    append(
        &path,
        json!({"type":"user", "sessionId":"synthetic",
        "message":{"role":"user", "content":"[Request interrupted by user]"}}),
    );
    append(
        &path,
        json!({"type":"last-prompt", "sessionId":"synthetic", "lastPrompt":"initial request"}),
    );
    let ProviderLastPromptRefresh::Current(Some(prompt)) = tracker.cached_refresh() else {
        panic!("current accepted input")
    };
    assert_eq!(prompt.text, "accepted queued request");
    assert_eq!(tracker.last_message().unwrap().text, prompt.text);
    let mut cold = ProviderLastPromptTracker::open_path(
        ProviderKind::Claude,
        "synthetic",
        path.clone(),
        Duration::ZERO,
    )
    .unwrap();
    assert_eq!(cold.refresh(), tracker.refresh());
    assert_eq!(cold.last_message(), tracker.last_message());
    append(&path, user(ProviderKind::Claude, "replacement request"));
    let ProviderLastPromptRefresh::Current(Some(prompt)) = tracker.cached_refresh() else {
        panic!("current replacement")
    };
    assert_eq!(prompt.text, "replacement request");
    assert_eq!(tracker.last_message().unwrap().text, prompt.text);
}

fn assistant(provider: ProviderKind, text: &str) -> Value {
    match provider {
        ProviderKind::Claude => json!({"type":"assistant", "sessionId":"synthetic",
            "timestamp":"2099-01-01T00:00:03Z", "message":{"role":"assistant", "content":[
                {"type":"thinking", "thinking":"internal reasoning"},
                {"type":"text", "text":text}, {"type":"tool_use", "name":"synthetic_tool"}]}}),
        ProviderKind::Codex => json!({"type":"response_item", "timestamp":"2099-01-01T00:00:03Z",
            "payload":{"type":"message", "role":"assistant", "content":[{"type":"output_text", "text":text}]}}),
    }
}

#[test]
fn newest_message_roles_order_caps_and_hygiene_match_cold_recovery() {
    for provider in [ProviderKind::Claude, ProviderKind::Codex] {
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("synthetic.jsonl");
        fs::write(&path, "").unwrap();
        append(&path, user(provider, "initial request"));
        let mut tracker = ProviderLastPromptTracker::open_path(
            provider,
            "synthetic",
            path.clone(),
            Duration::ZERO,
        )
        .unwrap();
        let text = format!(
            "answer\u{0}\u{1b}\t\n{}",
            "語".repeat(MAX_PROVIDER_PROMPT_BYTES)
        );
        append(&path, assistant(provider, &text));
        assert!(matches!(
            tracker.cached_refresh(),
            ProviderLastPromptRefresh::Current(_)
        ));
        let message = tracker.last_message().unwrap();
        assert_eq!(message.role, "assistant");
        assert_eq!(
            message.submitted_at.as_deref(),
            Some("2099-01-01T00:00:03Z")
        );
        assert!(message.truncated);
        assert!(message.text.len() <= MAX_PROVIDER_PROMPT_BYTES);
        assert!(!message.text.contains(['\0', '\u{1b}']));
        assert!(message.text.starts_with("answer\t\n"));
        let mut cold = ProviderLastPromptTracker::open_path(
            provider,
            "synthetic",
            path.clone(),
            Duration::ZERO,
        )
        .unwrap();
        assert_eq!(cold.last_message(), Some(message));
        assert_eq!(cold.refresh(), tracker.refresh());
        append(&path, user(provider, "later request"));
        assert!(matches!(
            tracker.cached_refresh(),
            ProviderLastPromptRefresh::Current(_)
        ));
        assert_eq!(tracker.last_message().unwrap().role, "user");
        assert_eq!(tracker.last_message().unwrap().text, "later request");
    }
}

#[test]
fn last_message_removes_image_wrappers_and_ignores_machine_records() {
    let tmp = tempfile::tempdir().unwrap();
    let path = tmp.path().join("synthetic.jsonl");
    fs::write(&path, "").unwrap();
    let mut tracker = ProviderLastPromptTracker::open_path(
        ProviderKind::Codex,
        "synthetic",
        path.clone(),
        Duration::ZERO,
    )
    .unwrap();
    append(
        &path,
        json!({"type":"response_item", "payload":{"type":"message", "role":"user",
        "internal_chat_message_metadata_passthrough":{"content_item_kinds":["user.text","user.image"]},
        "content":[{"type":"input_text","text":"<image name=[Image #1] path=\"/attachments/synthetic.png\">"},
            {"type":"input_image","image_url":"data:image/png;base64,AAAA"},
            {"type":"input_text","text":"</image>"}, {"type":"input_text","text":"question"}]}}),
    );
    append(
        &path,
        json!({"type":"response_item", "payload":{"type":"message", "role":"user",
        "content":[{"type":"input_text","text":"machine context"}]}}),
    );
    let ProviderLastPromptRefresh::Current(Some(prompt)) = tracker.cached_refresh() else {
        panic!("current")
    };
    assert_eq!(prompt.text, "question");
    assert_eq!(tracker.last_message().unwrap().text, prompt.text);
}
