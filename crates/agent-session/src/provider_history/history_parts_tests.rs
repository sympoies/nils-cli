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
                    ..Default::default()
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
    let serialized = serde_json::to_string(&masked_view(&parts)).unwrap();
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
    let serialized = serde_json::to_string(&masked_view(&parts)).unwrap();
    assert!(!serialized.contains("fixture-secret"));
    assert!(
        !serde_json::to_string(&parts)
            .unwrap()
            .contains("fixture-secret")
    );
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
    let input = serde_json::to_string(&masked_view(&parts)).unwrap();
    let both_forms = serde_json::to_string(&parts).unwrap();
    for secret in ["credential-value", "session-value", "another-secret"] {
        assert!(!input.contains(secret));
        assert!(
            !both_forms.contains(secret),
            "credential exposed in owner form"
        );
    }

    let mut result = fixture("codex").remove(3);
    result["payload"]["output"] = Value::String(
        "opened (src/private/config) path=/private/location/file cwd: /private/location [~/session.jsonl] \"/private/session/transcript.jsonl\", redirect>/private/redirect.log value,/private/comma.log trace=src/lib.rs:12:5 {\"arg\":\"/private/json/value\"}"
            .to_string(),
    );
    let (parts, _) = record_parts("codex", Some("0.134.0"), &result, true, &[], "fallback");
    let output = serde_json::to_string(&masked_view(&parts)).unwrap();
    for private_path in [
        "src/private/config",
        "/private/location/file",
        "/private/location",
        "~/session.jsonl",
        "/private/session/transcript.jsonl",
        "/private/redirect.log",
        "/private/comma.log",
        "src/lib.rs:12",
        "/private/json/value",
    ] {
        assert!(
            !output.contains(private_path),
            "path leaked: {private_path}"
        );
    }
    assert!(output.contains("[redacted]"));
    assert!(
        output.contains("opened"),
        "non-path text must survive: {output}"
    );
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
        paging_offset: 0,
        history_turn_id: None,
        prompt_request: None,
        row_class: None,
        compaction: None,
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
            owner_input_summary: None,
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
        owner_summary: None,
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
    let serialized = serde_json::to_string(&masked_view(&parts)).unwrap();
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
    let serialized = serde_json::to_string(&masked_view(&parts)).unwrap();
    assert!(!serialized.contains("/private/session/question.txt"));
}

#[test]
fn command_summaries_preserve_words_around_path_fragments() {
    assert_eq!(
        redact_text("ls -la src/ then done"),
        "ls -la [redacted] then done"
    );
    assert_eq!(
        redact_text("cat\tpath=src/lib.rs:12:5\nfinished"),
        "cat\t[redacted]\nfinished"
    );
    assert_eq!(redact_text("/private/path with spaces"), "[redacted]");
}

#[test]
fn owner_forms_fixture_preserves_masked_values_and_exposes_only_owner_paths() {
    let cases: Vec<Value> = serde_json::from_str(include_str!(
        "../../tests/fixtures/history-parts/owner-forms.json"
    ))
    .unwrap();
    for provider in ["codex", "claude"] {
        for case in &cases {
            let input = serde_json::json!({"description": case["value"]});
            let records = if provider == "codex" {
                vec![
                    serde_json::json!({"type":"response_item", "payload":{
                        "type":"function_call", "name":"inspect", "arguments": input.to_string()
                    }}),
                    serde_json::json!({"type":"response_item", "payload":{
                        "type":"function_call_output", "output": case["value"]
                    }}),
                ]
            } else {
                vec![
                    serde_json::json!({"type":"assistant", "version":"2.1.3", "message":{
                        "content":[
                            {"type":"tool_use", "name":"inspect", "input":input},
                            {"type":"tool_result", "content":case["value"]}
                        ]
                    }}),
                ]
            };
            let parts = project(provider, &records);
            let masked_input = serde_json::json!({"description":case["masked"]}).to_string();
            let owner_input = serde_json::json!({"description":case["owner"]}).to_string();
            let masked_result = case["masked"]
                .as_str()
                .map(str::to_string)
                .unwrap_or_else(|| case["masked"].to_string());
            let owner_result = case["owner"]
                .as_str()
                .map(str::to_string)
                .unwrap_or_else(|| case["owner"].to_string());
            assert_eq!(
                parts[0]["input_summary"], masked_input,
                "{provider}: {}",
                case["name"]
            );
            assert_eq!(
                parts[1]["summary"], masked_result,
                "{provider}: {}",
                case["name"]
            );
            if let Some(question) = case["value"].as_str() {
                let record = if provider == "codex" {
                    serde_json::json!({"type":"response_item", "payload":{
                        "type":"function_call", "name":"request_user_input",
                        "arguments":{"question":question}
                    }})
                } else {
                    serde_json::json!({"type":"assistant", "version":"2.1.3", "message":{
                        "content":[{"type":"tool_use", "name":"AskUserQuestion",
                            "input":{"questions":[{"question":question}]}}]
                    }})
                };
                let question_parts = project(provider, &[record]);
                let question_part = &question_parts[1];
                assert_eq!(
                    question_part["text"], case["masked"],
                    "{provider}: {}",
                    case["name"]
                );
                if case["masked"] == case["owner"] {
                    assert!(question_part.get("owner_text").is_none());
                } else {
                    assert_eq!(
                        question_part["owner_text"], case["owner"],
                        "{provider}: {}",
                        case["name"]
                    );
                }
            }
            for (part, field, masked, owner) in [
                (&parts[0], "owner_input_summary", masked_input, owner_input),
                (&parts[1], "owner_summary", masked_result, owner_result),
            ] {
                if masked == owner {
                    assert!(
                        part.get(field).is_none(),
                        "identical sibling must be omitted: {part}"
                    );
                } else {
                    assert_eq!(part[field], owner, "{provider}: {}", case["name"]);
                }
            }
        }
    }
}

#[test]
fn owner_question_text_is_optional_and_independently_bounded() {
    let question = format!("Open /workspace/{}", "界".repeat(MAX_QUESTION_CHARS));
    let record = serde_json::json!({"type":"response_item", "payload":{
        "type":"function_call", "name":"request_user_input", "arguments":{"question":question}
    }});
    let parts = project("codex", &[record]);
    assert_eq!(parts[1]["text"], "Open [redacted]");
    assert_eq!(
        parts[1]["owner_text"],
        question
            .chars()
            .take(MAX_QUESTION_CHARS)
            .collect::<String>()
    );
    assert_eq!(
        parts[0]["owner_input_summary"]
            .as_str()
            .unwrap()
            .chars()
            .count(),
        MAX_PART_SUMMARY_CHARS
    );
    let record = serde_json::json!({"type":"response_item", "payload":{
        "type":"function_call", "name":"request_user_input", "arguments":{"question":"Continue?"}
    }});
    let parts = project("codex", &[record]);
    assert!(parts[0].get("owner_input_summary").is_none());
    assert!(parts[1].get("owner_text").is_none());
}

#[test]
fn owner_result_bounds_use_owner_text_instead_of_masked_offsets() {
    let text = format!("read /workspace/{}", "界".repeat(MAX_PART_SUMMARY_CHARS));
    let record = serde_json::json!({"type":"response_item", "payload":{
        "type":"function_call_output", "output":text
    }});
    let parts = project("codex", &[record]);
    assert_eq!(parts[0]["summary"], "read [redacted]");
    assert_eq!(parts[0]["truncated"], false);
    assert_eq!(
        parts[0]["owner_summary"],
        text.chars()
            .take(MAX_PART_SUMMARY_CHARS)
            .collect::<String>()
    );
}

#[test]
fn existing_provider_fixtures_keep_byte_identical_masked_parts() {
    for (provider, expected) in [
        (
            "codex",
            include_str!("../../tests/fixtures/history-parts/codex-masked-parts.json"),
        ),
        (
            "claude",
            include_str!("../../tests/fixtures/history-parts/claude-masked-parts.json"),
        ),
    ] {
        let parts = project(provider, &fixture(provider));
        assert_eq!(
            serde_json::to_string(&masked_view(&parts)).unwrap(),
            expected.trim(),
            "{provider}"
        );
    }
}

fn masked_view(parts: &impl serde::Serialize) -> Value {
    let mut value = serde_json::to_value(parts).unwrap();
    for part in value.as_array_mut().unwrap() {
        let object = part.as_object_mut().unwrap();
        for key in ["owner_input_summary", "owner_summary", "owner_text"] {
            object.remove(key);
        }
    }
    value
}

#[test]
fn owner_forms_keep_credential_keys_masked_inside_path_objects() {
    let record = serde_json::json!({"type":"response_item", "payload":{
        "type":"function_call", "name":"inspect", "arguments":{
            "cwd":"/workspace/project", "file_path":{
                "path":"src/lib.rs", "client-secret":"synthetic-credential"
            }
        }
    }});
    let parts = project("codex", &[record]);
    assert_eq!(
        parts[0]["input_summary"],
        r#"{"cwd":"[redacted]","file_path":"[redacted]"}"#
    );
    assert_eq!(
        parts[0]["owner_input_summary"],
        r#"{"cwd":"/workspace/project","file_path":{"client-secret":"[redacted]","path":"src/lib.rs"}}"#
    );
    assert!(!parts[0].to_string().contains("synthetic-credential"));
}

#[test]
fn owner_siblings_are_omitted_when_independent_bounds_make_them_equal() {
    let text = format!("{} /workspace/project", "x".repeat(MAX_PART_SUMMARY_CHARS));
    let record = serde_json::json!({"type":"response_item", "payload":{
        "type":"function_call_output", "output":text
    }});
    let parts = project("codex", &[record]);
    assert_eq!(parts[0]["summary"], "x".repeat(MAX_PART_SUMMARY_CHARS));
    assert!(parts[0].get("owner_summary").is_none());
}

#[test]
fn owner_summaries_share_the_aggregate_message_byte_budget() {
    let record = serde_json::json!({"type":"assistant", "version":"2.1.3", "message":{
        "content":(0..40).map(|_| serde_json::json!({"type":"tool_result",
            "content":format!("read /workspace/{}", "界".repeat(MAX_PART_SUMMARY_CHARS))
        })).collect::<Vec<_>>()
    }});
    let parts = project("claude", &[record]);
    assert!(parts.len() > 1);
    assert!(parts.len() < 40);
    assert!(serde_json::to_vec(&parts).unwrap().len() <= MAX_PART_BYTES_PER_MESSAGE);
    assert!(
        parts
            .iter()
            .all(|part| part["summary"] == "read [redacted]")
    );
    assert!(parts.iter().all(|part| part["truncated"] == false));
    assert!(parts.iter().all(|part| {
        part["owner_summary"]
            .as_str()
            .is_some_and(|text| text.chars().count() <= MAX_PART_SUMMARY_CHARS)
    }));
}

#[test]
fn both_history_forms_mask_shared_bare_token_patterns() {
    let text = include_str!("../../tests/fixtures/history-parts/bare-token.txt").trim_end();
    let record = serde_json::json!({"type":"response_item", "payload":{
        "type":"function_call_output", "output":text
    }});
    let parts = project("codex", &[record]);
    assert_eq!(parts[0]["summary"], "inspect [redacted] then");
    assert!(parts[0].get("owner_summary").is_none());
    assert_eq!(owner_text(text), "inspect [redacted] then");
}
