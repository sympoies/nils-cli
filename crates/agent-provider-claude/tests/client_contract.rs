use agent_provider_claude::client::ClaudeApiClient;
use agent_provider_claude::config::ClaudeConfig;
use agent_runtime_core::schema::ProviderErrorCategory;
use nils_test_support::http::{HttpResponse, LoopbackServer};
use nils_test_support::{EnvGuard, GlobalStateLock};
use pretty_assertions::assert_eq;
use serde_json::json;
use std::net::TcpListener;

#[test]
fn client_posts_to_messages_endpoint_and_extracts_text() {
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
                    { "type": "text", "text": "hello from claude" }
                ],
                "model": "claude-sonnet-4-5-20250929"
            })
            .to_string(),
        )
        .with_header("x-request-id", "req_abc"),
    );

    let _api_key = EnvGuard::set(&lock, "ANTHROPIC_API_KEY", "test-key");
    let _base_url = EnvGuard::set(&lock, "ANTHROPIC_BASE_URL", server.url().as_str());
    let _retry_max = EnvGuard::set(&lock, "CLAUDE_RETRY_MAX", "0");
    let _model = EnvGuard::set(&lock, "CLAUDE_MODEL", "claude-sonnet-4-5-20250929");
    let config = ClaudeConfig::from_env().expect("config");
    let client = ClaudeApiClient::new(config).expect("client");

    let result = client.execute_prompt("ping", None).expect("execute");
    assert_eq!(result.text, "hello from claude");
    assert_eq!(result.request_id.as_deref(), Some("req_abc"));

    let requests = server.take_requests();
    assert_eq!(requests.len(), 1);
    let req = &requests[0];
    assert_eq!(req.method, "POST");
    assert_eq!(req.path, "/v1/messages");
    assert_eq!(req.header_value("x-api-key").as_deref(), Some("test-key"));
    assert_eq!(
        req.header_value("anthropic-version").as_deref(),
        Some("2023-06-01")
    );
    let body = req.body_text();
    assert!(body.contains("\"messages\""));
    assert!(body.contains("ping"));
}

#[test]
fn client_maps_rate_limit_error_to_retryable_category() {
    let lock = GlobalStateLock::new();
    let server = LoopbackServer::new().expect("loopback server");
    server.add_route(
        "POST",
        "/v1/messages",
        HttpResponse::new(
            429,
            json!({
                "type": "error",
                "error": {
                    "type": "rate_limit_error",
                    "message": "Too many requests"
                }
            })
            .to_string(),
        ),
    );

    let _api_key = EnvGuard::set(&lock, "ANTHROPIC_API_KEY", "test-key");
    let _base_url = EnvGuard::set(&lock, "ANTHROPIC_BASE_URL", server.url().as_str());
    let _retry_max = EnvGuard::set(&lock, "CLAUDE_RETRY_MAX", "0");
    let config = ClaudeConfig::from_env().expect("config");
    let client = ClaudeApiClient::new(config).expect("client");

    let error = client
        .execute_prompt("ping", None)
        .expect_err("rate limit expected");
    assert_eq!(error.category, ProviderErrorCategory::RateLimit);
    assert_eq!(error.code, "rate-limited");
    assert_eq!(error.retryable, true);
}

#[test]
fn client_retries_only_retryable_failures() {
    let lock = GlobalStateLock::new();
    let server = LoopbackServer::new().expect("loopback server");
    server.add_route(
        "POST",
        "/v1/messages",
        HttpResponse::new(
            429,
            json!({
                "type": "error",
                "error": {
                    "type": "rate_limit_error",
                    "message": "Too many requests"
                }
            })
            .to_string(),
        ),
    );

    let _api_key = EnvGuard::set(&lock, "ANTHROPIC_API_KEY", "test-key");
    let _base_url = EnvGuard::set(&lock, "ANTHROPIC_BASE_URL", server.url().as_str());
    let _retry_max = EnvGuard::set(&lock, "CLAUDE_RETRY_MAX", "1");
    let config = ClaudeConfig::from_env().expect("config");
    let client = ClaudeApiClient::new(config).expect("client");

    let error = client
        .execute_prompt("ping", None)
        .expect_err("rate limit expected");
    assert_eq!(error.category, ProviderErrorCategory::RateLimit);
    assert_eq!(error.code, "rate-limited");
    assert_eq!(error.retryable, true);

    let requests = server.take_requests();
    assert_eq!(requests.len(), 2);
}

#[test]
fn client_does_not_retry_non_retryable_failures() {
    let lock = GlobalStateLock::new();
    let server = LoopbackServer::new().expect("loopback server");
    server.add_route(
        "POST",
        "/v1/messages",
        HttpResponse::new(
            400,
            json!({
                "type": "error",
                "error": {
                    "type": "invalid_request_error",
                    "message": "invalid request"
                }
            })
            .to_string(),
        ),
    );

    let _api_key = EnvGuard::set(&lock, "ANTHROPIC_API_KEY", "test-key");
    let _base_url = EnvGuard::set(&lock, "ANTHROPIC_BASE_URL", server.url().as_str());
    let _retry_max = EnvGuard::set(&lock, "CLAUDE_RETRY_MAX", "3");
    let config = ClaudeConfig::from_env().expect("config");
    let client = ClaudeApiClient::new(config).expect("client");

    let error = client
        .execute_prompt("ping", None)
        .expect_err("validation error expected");
    assert_eq!(error.category, ProviderErrorCategory::Validation);
    assert_eq!(error.code, "invalid-request");
    assert_eq!(error.retryable, false);

    let requests = server.take_requests();
    assert_eq!(requests.len(), 1);
}

#[test]
fn client_maps_gateway_timeout_http_error() {
    let lock = GlobalStateLock::new();
    let server = LoopbackServer::new().expect("loopback server");
    server.add_route(
        "POST",
        "/v1/messages",
        HttpResponse::new(
            504,
            json!({
                "type": "error",
                "error": {
                    "type": "timeout_error",
                    "message": "gateway timeout"
                }
            })
            .to_string(),
        ),
    );

    let _api_key = EnvGuard::set(&lock, "ANTHROPIC_API_KEY", "test-key");
    let _base_url = EnvGuard::set(&lock, "ANTHROPIC_BASE_URL", server.url().as_str());
    let _retry_max = EnvGuard::set(&lock, "CLAUDE_RETRY_MAX", "0");
    let config = ClaudeConfig::from_env().expect("config");
    let client = ClaudeApiClient::new(config).expect("client");

    let error = client
        .execute_prompt("ping", None)
        .expect_err("timeout expected");
    assert_eq!(error.category, ProviderErrorCategory::Timeout);
    assert_eq!(error.code, "request-timeout");
    assert_eq!(error.retryable, true);
}

#[test]
fn client_maps_server_errors_to_unavailable() {
    let lock = GlobalStateLock::new();
    let server = LoopbackServer::new().expect("loopback server");
    server.add_route(
        "POST",
        "/v1/messages",
        HttpResponse::new(
            503,
            json!({
                "type": "error",
                "error": {
                    "type": "overloaded_error",
                    "message": "service unavailable"
                }
            })
            .to_string(),
        ),
    );

    let _api_key = EnvGuard::set(&lock, "ANTHROPIC_API_KEY", "test-key");
    let _base_url = EnvGuard::set(&lock, "ANTHROPIC_BASE_URL", server.url().as_str());
    let _retry_max = EnvGuard::set(&lock, "CLAUDE_RETRY_MAX", "0");
    let config = ClaudeConfig::from_env().expect("config");
    let client = ClaudeApiClient::new(config).expect("client");

    let error = client
        .execute_prompt("ping", None)
        .expect_err("unavailable expected");
    assert_eq!(error.category, ProviderErrorCategory::Unavailable);
    assert_eq!(error.code, "upstream-unavailable");
    assert_eq!(error.retryable, true);
}

#[test]
fn client_maps_connect_failure_to_network_error() {
    let lock = GlobalStateLock::new();
    let listener = TcpListener::bind("127.0.0.1:0").expect("bind available port");
    let unused_addr = listener.local_addr().expect("unused addr");
    drop(listener);

    let _api_key = EnvGuard::set(&lock, "ANTHROPIC_API_KEY", "test-key");
    let _base_url = EnvGuard::set(
        &lock,
        "ANTHROPIC_BASE_URL",
        &format!("http://{unused_addr}"),
    );
    let _retry_max = EnvGuard::set(&lock, "CLAUDE_RETRY_MAX", "0");
    let config = ClaudeConfig::from_env().expect("config");
    let client = ClaudeApiClient::new(config).expect("client");

    let error = client
        .execute_prompt("ping", None)
        .expect_err("network error expected");
    assert_eq!(error.category, ProviderErrorCategory::Network);
    assert_eq!(error.code, "network-error");
    assert_eq!(error.retryable, true);
}
