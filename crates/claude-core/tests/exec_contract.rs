use claude_core::exec::execute_task;
use nils_test_support::http::{HttpResponse, LoopbackServer};
use nils_test_support::{EnvGuard, GlobalStateLock};
use pretty_assertions::assert_eq;
use serde_json::json;

#[test]
fn execute_task_returns_text_and_request_id_metadata() {
    let lock = GlobalStateLock::new();
    let server = LoopbackServer::new().expect("loopback server");
    server.add_route(
        "POST",
        "/v1/messages",
        HttpResponse::new(
            200,
            json!({
                "id": "msg_123",
                "type": "message",
                "role": "assistant",
                "content": [
                    { "type": "text", "text": "core execute ok" }
                ],
                "model": "claude-sonnet-4-5-20250929"
            })
            .to_string(),
        )
        .with_header("x-request-id", "req_001"),
    );

    let _api_key = EnvGuard::set(&lock, "ANTHROPIC_API_KEY", "test-key");
    let _base_url = EnvGuard::set(&lock, "ANTHROPIC_BASE_URL", server.url().as_str());
    let _retry_max = EnvGuard::set(&lock, "CLAUDE_RETRY_MAX", "0");

    let result = execute_task("prompt: hello", None, None).expect("execute");
    assert_eq!(result.stdout, "core execute ok");
    assert!(result.stderr.contains("request_id=req_001"));
}

#[test]
fn execute_task_rejects_blank_input() {
    let error = execute_task("", Some(""), None).expect_err("missing task");
    assert_eq!(error.code, "missing-task");
}
