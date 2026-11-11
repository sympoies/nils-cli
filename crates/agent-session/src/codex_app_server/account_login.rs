//! External-auth login with credential-free, closed-set failure causes.

use super::*;

/// Resolve the current broker credential on every attempt, including after an
/// explicit relaunch. Only the live app-server is mutated here; callers own
/// the incarnation/revision-fenced durable completion.
pub(super) async fn drive_external_auth_login(
    websocket: &mut tokio_tungstenite::WebSocketStream<UnixStream>,
    request_id: &mut u64,
    account: &str,
) -> Result<(), &'static str> {
    let resolve_account = account.to_string();
    let credentials = tokio::task::spawn_blocking(move || {
        crate::codex_account::resolve_account(&resolve_account, false)
    })
    .await
    .map_err(|_| "broker_worker_failed")?
    .map_err(|error| match error.code() {
        "codex-account-broker-rejected" => "broker_resolve_rejected",
        "codex-account-broker-timeout" => "broker_resolve_timeout",
        "codex-account-broker-unavailable" => "broker_resolve_unavailable",
        "codex-account-broker-invalid-config" => "broker_invalid_config",
        "codex-account-broker-invalid-response" => "broker_invalid_response",
        "codex-account-unsupported" => "broker_unconfigured",
        _ => "broker_resolve_failed",
    })?;

    *request_id = request_id.saturating_add(1);
    send_json(
        websocket,
        external_auth_login_request(
            *request_id,
            &credentials.access_token,
            &credentials.chatgpt_account_id,
            credentials.chatgpt_plan_type.as_deref(),
        ),
    )
    .await
    .map_err(|_| "login_write_failed")?;
    let result = tokio::time::timeout(
        CONTROL_RESPONSE_TIMEOUT,
        prompt_submit::receive_response(websocket, *request_id, None, None),
    )
    .await
    .map_err(|_| "login_timeout")?
    .map_err(|error| match error {
        prompt_submit::PromptResponseFailure::Rejected(_) => "login_rejected",
        prompt_submit::PromptResponseFailure::NotConfirmed(_) => "login_invalid_response",
        prompt_submit::PromptResponseFailure::AuthRefresh(_) => "login_refresh_failed",
        prompt_submit::PromptResponseFailure::Other(_) => "login_response_failed",
    })?;
    if result.get("type").and_then(Value::as_str) != Some("chatgptAuthTokens") {
        return Err("login_invalid_response");
    }
    Ok(())
}
