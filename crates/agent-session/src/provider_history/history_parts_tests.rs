use super::*;
use pretty_assertions::assert_eq;

fn fixture(name: &str) -> Vec<Value> {
    let contents = match name {
        "codex" => include_str!("../../tests/fixtures/history-parts/codex-response-item-v1.jsonl"),
        "claude" => include_str!("../../tests/fixtures/history-parts/claude-code-2.1.x.jsonl"),
        _ => panic!("unknown fixture"),
    };
    contents
        .lines()
        .map(|line| serde_json::from_str(line).unwrap())
        .collect()
}

fn project(provider: &str, values: &[Value]) -> Vec<Value> {
    values
        .iter()
        .enumerate()
        .flat_map(|(index, value)| {
            let images = super::super::image_sources(value);
            let descriptors: Vec<_> = images
                .iter()
                .enumerate()
                .map(|(image_index, _)| super::super::HistoryImage {
                    id: format!("opaque-image-{index}-{image_index}"),
                })
                .collect();
            record_parts(
                provider,
                (provider == "codex").then_some("0.134.0"),
                value,
                true,
                &descriptors,
                &format!("fallback-{index}"),
            )
            .0
            .into_iter()
            .map(|part| serde_json::to_value(part).unwrap())
        })
        .collect()
}

#[test]
fn codex_response_item_v1_fixture_projects_all_typed_parts_with_safe_summaries() {
    let records = fixture("codex");
    let parts = project("codex", &records);
    let kinds: Vec<_> = parts
        .iter()
        .filter_map(|part| part["kind"].as_str())
        .collect();
    assert_eq!(
        kinds,
        [
            "text",
            "image",
            "text",
            "tool_call",
            "tool_result",
            "tool_call",
            "question"
        ]
    );
    assert_eq!(parts[0]["id"], "codex-user-1");
    assert_eq!(parts[0]["timestamp"], "2026-10-23T00:00:01Z");
    assert_eq!(parts[1]["ref"], "opaque-image-1-0");
    assert_eq!(parts[1]["mime"], "image/png");
    assert_eq!(parts[1]["bytes"], 1);
    assert_eq!(parts[2]["text"], "and report what you see");
    let serialized = serde_json::to_string(&parts).unwrap();
    assert!(!serialized.contains("/workspace/fixture"));
    assert!(
        parts[3]["input_summary"]
            .as_str()
            .unwrap()
            .contains("\"file_path\":\"[redacted]\"")
    );
    assert_eq!(parts[5]["name"], "request_user_input");
    assert_eq!(parts[6]["source"], "request_user_input");
    assert_eq!(parts[6]["text"], "Continue with the change?");
    assert_eq!(parts[3]["id"], "codex-call-1");
    assert_eq!(parts[4]["id"], "codex-result-1");
    assert!(
        record_parts("codex", None, &records[1], true, &[], "fallback")
            .0
            .is_empty()
    );
}

#[test]
fn claude_code_2_1_fixture_projects_parts_and_unknown_versions_fail_closed() {
    let records = fixture("claude");
    let parts = project("claude", &records);
    let kinds: Vec<_> = parts
        .iter()
        .filter_map(|part| part["kind"].as_str())
        .collect();
    assert_eq!(
        kinds,
        [
            "text",
            "text",
            "image",
            "tool_call",
            "tool_result",
            "image",
            "tool_call",
            "question"
        ]
    );
    assert_eq!(parts[6]["name"], "AskUserQuestion");
    assert_eq!(parts[7]["source"], "ask_user_question");
    assert_eq!(parts[3]["id"], "claude-call-1");
    assert_eq!(parts[4]["id"], "claude-result-1");
    assert_eq!(parts[3]["call_id"], "tool-1");
    assert_eq!(parts[4]["call_id"], "tool-1");
    let serialized = serde_json::to_string(&parts).unwrap();
    assert!(!serialized.contains("fixture-secret"));
    assert!(!serialized.contains("/workspace/fixture"));

    let mut unknown = records[0].clone();
    unknown["version"] = Value::String("2.2.0".to_string());
    assert!(project("claude", &[unknown]).is_empty());
}

#[test]
fn tool_result_summary_is_bounded_and_reports_truncation() {
    let mut value = fixture("codex").remove(3);
    value["payload"]["output"] = Value::String("x".repeat(MAX_PART_SUMMARY_CHARS + 8));
    let (parts, _) = record_parts("codex", Some("0.134.0"), &value, true, &[], "fallback");
    let serialized = serde_json::to_value(parts).unwrap();
    assert_eq!(serialized[0]["kind"], "tool_result");
    assert_eq!(
        serialized[0]["summary"].as_str().unwrap().chars().count(),
        MAX_PART_SUMMARY_CHARS
    );
    assert_eq!(serialized[0]["truncated"], true);

    let mut call = fixture("codex").remove(2);
    call["payload"]["arguments"] = Value::String(
        serde_json::json!({"description": "x".repeat(MAX_PART_SUMMARY_CHARS * 2)}).to_string(),
    );
    let (parts, _) = record_parts("codex", Some("0.134.0"), &call, true, &[], "fallback");
    let input_summary = serde_json::to_value(parts).unwrap()[0]["input_summary"]
        .as_str()
        .unwrap()
        .to_string();
    assert_eq!(input_summary.chars().count(), MAX_PART_SUMMARY_CHARS);
}

#[test]
fn summaries_redact_credentials_and_relative_paths() {
    let mut call = fixture("codex").remove(2);
    call["payload"]["arguments"] = Value::String(
        serde_json::json!({
            "api_key": "credential-value",
            "headers": {"Cookie": "session-value"},
            "credential": "another-secret",
        })
        .to_string(),
    );
    let (parts, _) = record_parts("codex", Some("0.134.0"), &call, true, &[], "fallback");
    let input = serde_json::to_string(&parts).unwrap();
    for secret in ["credential-value", "session-value", "another-secret"] {
        assert!(!input.contains(secret));
    }

    let mut result = fixture("codex").remove(3);
    result["payload"]["output"] = Value::String(
        "opened (src/private/config) path=/private/location/file cwd: /private/location [~/session.jsonl] \"/private/session/transcript.jsonl\","
            .to_string(),
    );
    let (parts, _) = record_parts("codex", Some("0.134.0"), &result, true, &[], "fallback");
    let output = serde_json::to_string(&parts).unwrap();
    for private_path in [
        "src/private/config",
        "/private/location/file",
        "/private/location",
        "~/session.jsonl",
        "/private/session/transcript.jsonl",
    ] {
        assert!(!output.contains(private_path));
    }
    assert!(output.contains("[redacted]"));
}

#[test]
fn typed_parts_are_bounded_per_message() {
    let mut value = fixture("claude").remove(0);
    value["message"]["content"] = Value::Array(
        (0..MAX_PARTS_PER_MESSAGE + 20)
            .map(|_| serde_json::json!({"type":"text", "text":"part"}))
            .collect(),
    );
    let (parts, _) = record_parts("claude", None, &value, true, &[], "fallback");
    assert_eq!(parts.len(), MAX_PARTS_PER_MESSAGE);
}

#[test]
fn matching_tool_results_set_call_status() {
    let mut messages = vec![super::super::HistoryMessage {
        id: "message-call".to_string(),
        role: "assistant".to_string(),
        text: String::new(),
        timestamp: None,
        images: Vec::new(),
        parts: Some(vec![HistoryPart::ToolCall {
            id: "tool-1".to_string(),
            call_id: Some("tool-1".to_string()),
            timestamp: None,
            name: "Read".to_string(),
            input_summary: "{}".to_string(),
            status: "unknown".to_string(),
            started_at: None,
            ended_at: None,
        }]),
        human_prompt: false,
    }];
    let results = [HistoryPart::ToolResult {
        id: "tool-1".to_string(),
        call_id: Some("tool-1".to_string()),
        timestamp: Some("2026-10-23T00:00:03Z".to_string()),
        summary: "ok".to_string(),
        truncated: false,
        is_error: false,
    }];
    correlate_tool_status(&mut messages, &results);
    let serialized = serde_json::to_value(&messages).unwrap();
    assert_eq!(serialized[0]["parts"][0]["status"], "completed");
    assert_eq!(
        serialized[0]["parts"][0]["ended_at"],
        "2026-10-23T00:00:03Z"
    );
}

#[test]
fn status_context_finds_results_after_the_current_page() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("transcript.jsonl");
    let records = [
        serde_json::json!({
            "type":"assistant", "uuid":"call-row", "version":"2.1.3",
            "message":{"content":[{"type":"tool_use","id":"tool-1","name":"Read","input":{}}]}
        }),
        serde_json::json!({
            "type":"user", "uuid":"result-row", "version":"2.1.3",
            "message":{"content":[{"type":"tool_result","tool_use_id":"tool-1","content":"ok"}]}
        }),
    ];
    let content = records
        .iter()
        .map(serde_json::Value::to_string)
        .collect::<Vec<_>>()
        .join("\n");
    std::fs::write(&path, content).unwrap();

    let results = super::super::matching_tool_results(&path, "claude", None, 0);
    assert!(results.iter().any(|part| matches!(
        part,
        HistoryPart::ToolResult {
            call_id: Some(call_id),
            is_error: false,
            ..
        } if call_id == "tool-1"
    )));
}

#[test]
fn typed_parts_have_an_aggregate_serialized_byte_bound() {
    let mut value = fixture("claude").remove(0);
    value["message"]["content"] = Value::Array(
        (0..8)
            .map(|_| serde_json::json!({"type":"text", "text":"x".repeat(64 * 1024)}))
            .collect(),
    );
    let (parts, _) = record_parts("claude", None, &value, true, &[], "fallback");
    assert!(serde_json::to_vec(&parts).unwrap().len() <= MAX_PART_BYTES_PER_MESSAGE);
}

#[test]
fn tool_ids_fall_back_when_unbounded_or_path_like() {
    let mut call = fixture("claude").remove(1);
    call["message"]["content"][2]["id"] = Value::String("/private/session/tool".to_string());
    let (parts, _) = record_parts("claude", None, &call, true, &[], "fallback");
    let serialized = serde_json::to_string(&parts).unwrap();
    assert!(!serialized.contains("/private/session/tool"));
    assert!(serialized.contains("claude-call-1"));

    call["timestamp"] = Value::String("x".repeat(129));
    let (parts, _) = record_parts("claude", None, &call, true, &[], "fallback");
    assert!(
        !serde_json::to_string(&parts)
            .unwrap()
            .contains(&"x".repeat(129))
    );
    assert_eq!(
        projected_timestamp(&serde_json::json!({"timestamp":"/private/time"})),
        None
    );

    let mut question = fixture("claude").remove(4);
    question["message"]["content"][0]["input"]["questions"][0]["question"] =
        Value::String("Open (/private/session/question.txt)?".to_string());
    let (parts, _) = record_parts("claude", None, &question, true, &[], "fallback");
    let serialized = serde_json::to_string(&parts).unwrap();
    assert!(!serialized.contains("/private/session/question.txt"));
}
