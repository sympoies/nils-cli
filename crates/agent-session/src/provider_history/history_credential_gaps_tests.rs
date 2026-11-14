use super::{MAX_PART_BYTES_PER_MESSAGE, Value, owner_text, project, redact_text};
use pretty_assertions::assert_eq;
use serde_json::json;

fn assert_forms(input: &str, masked: &str, owner: &str) {
    // Evaluate both forms before asserting so RED exposes either masking gap.
    assert_eq!(
        (redact_text(input), owner_text(input)),
        (masked.into(), owner.into())
    );
    assert_eq!(
        redact_text(masked),
        masked,
        "masked form must be idempotent"
    );
    assert_eq!(owner_text(owner), owner, "owner form must be idempotent");
    for provider in ["codex", "claude"] {
        let arguments = json!({"description": input});
        let records = if provider == "codex" {
            vec![
                json!({"type":"response_item", "payload":{
                    "type":"function_call", "name":"inspect", "arguments":arguments.to_string()
                }}),
                json!({"type":"response_item", "payload":{
                    "type":"function_call_output", "output":input
                }}),
            ]
        } else {
            vec![json!({"type":"assistant", "version":"2.1.3", "message":{
                "content":[
                    {"type":"tool_use", "name":"inspect", "input":arguments},
                    {"type":"tool_result", "content":input}
                ]
            }})]
        };
        let parts = project(provider, &records);
        assert_eq!(parts.len(), 2, "{provider}");
        let input_summary = &parts[0]["input_summary"];
        let owner_input = parts[0].get("owner_input_summary").unwrap_or(input_summary);
        let parse = |value: &Value| serde_json::from_str::<Value>(value.as_str().unwrap()).unwrap();
        assert_eq!(
            parse(input_summary),
            json!({"description":masked}),
            "{provider}"
        );
        assert_eq!(
            parse(owner_input),
            json!({"description":owner}),
            "{provider}"
        );
        assert_eq!(parts[1]["summary"], masked, "{provider}");
        assert_eq!(
            parts[1]
                .get("owner_summary")
                .unwrap_or(&parts[1]["summary"]),
            owner,
            "{provider}"
        );
        assert!(serde_json::to_vec(&parts).unwrap().len() <= MAX_PART_BYTES_PER_MESSAGE);
    }
}

#[test]
fn outer_double_quote_suffix_is_part_of_the_credential() {
    assert_forms(
        "echo \"token=SYNTHETIC\"SUFFIX then",
        "echo \"token=[redacted]\" then",
        "echo \"token=[redacted]\" then",
    );
}

#[test]
fn outer_list_quote_suffix_is_part_of_the_credential() {
    assert_forms(
        "['password=SYNTHETIC'SUFFIX']",
        "['password=[redacted]']",
        "['password=[redacted]']",
    );
}

#[test]
fn query_quoted_suffix_is_part_of_the_credential() {
    assert_forms(
        "fetch https://example.invalid/cb?token=SYNTHETIC\"SUFFIX\"&view=files then",
        "fetch https://example.invalid/cb?token=[redacted]&view=files then",
        "fetch https://example.invalid/cb?token=[redacted]&view=files then",
    );
}

#[test]
fn encoded_query_assignment_is_masked() {
    assert_forms(
        "fetch https://example.invalid/cb?token%3DSYNTHETIC&view=files then",
        "fetch https://example.invalid/cb?token%3D[redacted]&view=files then",
        "fetch https://example.invalid/cb?token%3D[redacted]&view=files then",
    );
}

#[test]
fn relative_url_query_credential_is_masked() {
    assert_forms(
        "fetch /cb?client_secret=SYNTHETIC&view=files then",
        "fetch /cb?client_secret=[redacted]&view=files then",
        "fetch /cb?client_secret=[redacted]&view=files then",
    );
    assert_forms(
        "fetch /cb?token=SYNTHETIC#section then",
        "fetch /cb?token=[redacted]#section then",
        "fetch /cb?token=[redacted]#section then",
    );
}

#[test]
fn ordinary_wrappers_and_noncredential_queries_are_preserved() {
    for text in [
        "echo \"status=ready\"SUFFIX then",
        "['status=ready'SUFFIX']",
        "question?status=ready then",
        "/cb?view=files",
        "tokenization is ordinary text",
    ] {
        assert_forms(text, text, text);
    }
    assert_forms(
        "https://example.invalid/cb?status%3Dready&view=files",
        "https://example.invalid/cb?status%3Dready&view=files",
        "https://example.invalid/cb?status%3Dready&view=files",
    );
}
