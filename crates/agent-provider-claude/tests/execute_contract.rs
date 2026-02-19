use agent_provider_claude::ClaudeProviderAdapter;
use agent_runtime_core::provider::ProviderAdapterV1;
use agent_runtime_core::schema::{ExecuteRequest, ProviderErrorCategory};
use nils_test_support::http::{HttpResponse, LoopbackServer};
use nils_test_support::{EnvGuard, GlobalStateLock};
use pretty_assertions::assert_eq;
use serde_json::json;

#[test]
fn execute_success_returns_text_output() {
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
                    { "type": "text", "text": "execution ok" }
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
    let adapter = ClaudeProviderAdapter::new();

    let response = adapter
        .execute(ExecuteRequest::new("prompt: say hello"))
        .expect("execute");
    assert_eq!(response.exit_code, 0);
    assert_eq!(response.stdout, "execution ok");
    assert!(response.stderr.contains("request_id=req_001"));

    let requests = server.take_requests();
    assert_eq!(requests.len(), 1);
    let body = requests[0].body_text();
    assert!(body.contains("say hello"));
}

#[test]
fn execute_expands_advice_prompt_template() {
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
                    { "type": "text", "text": "advice response" }
                ],
                "model": "claude-sonnet-4-5-20250929"
            })
            .to_string(),
        ),
    );

    let _api_key = EnvGuard::set(&lock, "ANTHROPIC_API_KEY", "test-key");
    let _base_url = EnvGuard::set(&lock, "ANTHROPIC_BASE_URL", server.url().as_str());
    let _retry_max = EnvGuard::set(&lock, "CLAUDE_RETRY_MAX", "0");
    let adapter = ClaudeProviderAdapter::new();

    let _ = adapter
        .execute(ExecuteRequest::new("advice: improve rust test reliability"))
        .expect("execute");
    let requests = server.take_requests();
    assert_eq!(requests.len(), 1);
    let body = requests[0].body_text();
    assert!(body.contains("senior software engineer"));
    assert!(body.contains("improve rust test reliability"));
}

#[test]
fn execute_maps_auth_http_error() {
    let lock = GlobalStateLock::new();
    let server = LoopbackServer::new().expect("loopback server");
    server.add_route(
        "POST",
        "/v1/messages",
        HttpResponse::new(
            401,
            json!({
                "type": "error",
                "error": {
                    "type": "authentication_error",
                    "message": "invalid api key"
                }
            })
            .to_string(),
        ),
    );

    let _api_key = EnvGuard::set(&lock, "ANTHROPIC_API_KEY", "test-key");
    let _base_url = EnvGuard::set(&lock, "ANTHROPIC_BASE_URL", server.url().as_str());
    let _retry_max = EnvGuard::set(&lock, "CLAUDE_RETRY_MAX", "0");
    let adapter = ClaudeProviderAdapter::new();

    let error = adapter
        .execute(ExecuteRequest::new("prompt: ping"))
        .expect_err("auth error expected");
    assert_eq!(error.category, ProviderErrorCategory::Auth);
    assert_eq!(error.code, "auth-failed");
}
