use nils_test_support::bin;
use nils_test_support::cmd::{self, CmdOptions, CmdOutput};
use nils_test_support::http::{HttpResponse, LoopbackServer};
use pretty_assertions::assert_eq;
use serde_json::json;
use std::path::PathBuf;

fn claude_cli_bin() -> PathBuf {
    bin::resolve("claude-cli")
}

fn run_with(args: &[&str], options: CmdOptions) -> CmdOutput {
    let bin = claude_cli_bin();
    cmd::run_with(&bin, args, &options)
}

#[test]
fn agent_prompt_executes_via_claude_core_runtime() {
    let server = LoopbackServer::new().expect("loopback");
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
                    { "type": "text", "text": "prompt ok" }
                ],
                "model": "claude-sonnet-4-5-20250929"
            })
            .to_string(),
        )
        .with_header("x-request-id", "req_001"),
    );

    let output = run_with(
        &["agent", "prompt", "hello", "world"],
        CmdOptions::default()
            .with_env("ANTHROPIC_API_KEY", "test-key")
            .with_env("ANTHROPIC_BASE_URL", server.url().as_str())
            .with_env("CLAUDE_RETRY_MAX", "0"),
    );

    assert_eq!(output.code, 0, "stderr={}", output.stderr_text());
    assert!(output.stdout_text().contains("prompt ok"));
    assert!(output.stderr_text().contains("request_id=req_001"));

    let requests = server.take_requests();
    assert_eq!(requests.len(), 1);
    assert!(requests[0].body_text().contains("hello world"));
}

#[test]
fn agent_advice_and_knowledge_render_templates() {
    let server = LoopbackServer::new().expect("loopback");
    server.add_route(
        "POST",
        "/v1/messages",
        HttpResponse::new(
            200,
            json!({
                "id": "msg_1",
                "type": "message",
                "role": "assistant",
                "content": [
                    { "type": "text", "text": "advice ok" }
                ],
                "model": "claude-sonnet-4-5-20250929"
            })
            .to_string(),
        ),
    );
    server.add_route(
        "POST",
        "/v1/messages",
        HttpResponse::new(
            200,
            json!({
                "id": "msg_2",
                "type": "message",
                "role": "assistant",
                "content": [
                    { "type": "text", "text": "knowledge ok" }
                ],
                "model": "claude-sonnet-4-5-20250929"
            })
            .to_string(),
        ),
    );

    let options = CmdOptions::default()
        .with_env("ANTHROPIC_API_KEY", "test-key")
        .with_env("ANTHROPIC_BASE_URL", server.url().as_str())
        .with_env("CLAUDE_RETRY_MAX", "0");

    let advice = run_with(
        &["agent", "advice", "improve", "rust", "tests"],
        options.clone(),
    );
    assert_eq!(advice.code, 0, "stderr={}", advice.stderr_text());

    let knowledge = run_with(&["agent", "knowledge", "eventual", "consistency"], options);
    assert_eq!(knowledge.code, 0, "stderr={}", knowledge.stderr_text());

    let requests = server.take_requests();
    assert_eq!(requests.len(), 2);
    assert!(requests[0].body_text().contains("senior software engineer"));
    assert!(requests[0].body_text().contains("improve rust tests"));
    assert!(
        requests[1]
            .body_text()
            .contains("Explain the following concept clearly")
    );
    assert!(requests[1].body_text().contains("eventual consistency"));
}

#[test]
fn agent_commands_require_non_empty_arguments() {
    let prompt = run_with(&["agent", "prompt"], CmdOptions::default());
    assert_eq!(prompt.code, 64);
    assert!(prompt.stderr_text().contains("missing prompt"));

    let advice = run_with(&["agent", "advice"], CmdOptions::default());
    assert_eq!(advice.code, 64);
    assert!(advice.stderr_text().contains("missing question"));

    let knowledge = run_with(&["agent", "knowledge"], CmdOptions::default());
    assert_eq!(knowledge.code, 64);
    assert!(knowledge.stderr_text().contains("missing concept"));
}
