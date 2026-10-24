//! Shared HTTP error mapping, including pre-dispatch authentication quarantine.
use super::*;

pub(crate) fn envelope_err(err: CliError) -> Response {
    let session_id = err
        .details()
        .and_then(|d| d["session_id"].as_str())
        .map(str::to_owned);
    let retained_error = err.clone();
    let data = err.into_inner();
    let status = match data.code.as_str() {
        "session-not-found"
        | "message-not-found"
        | "retitle-v3-operation-not-found"
        | "history-session-not-found" => StatusCode::NOT_FOUND,
        "mailbox-service-forbidden" => StatusCode::FORBIDDEN,
        "coordination-unauthorized" | "mailbox-service-unauthorized" => StatusCode::UNAUTHORIZED,
        "rate-limited" | "retitle-provider-rate-limited" => StatusCode::TOO_MANY_REQUESTS,
        "wait-timeout" | "retitle-provider-timeout" => StatusCode::REQUEST_TIMEOUT,
        "session-capacity-exceeded"
        | "session-exists"
        | "shell-incarnation-conflict"
        | "shell-not-running"
        | "shell-name-conflict"
        | "shell-runtime-changed"
        | "title-revision-conflict"
        | "title-state-conflict"
        | "session-incarnation-conflict"
        | "maintenance-preview-stale"
        | "maintenance-session-busy"
        | "claim-conflict"
        | "claim-revision-conflict"
        | "operation-revision-conflict"
        | "activity-revision-conflict"
        | "message-revision-conflict"
        | "idempotency-conflict"
        | "idempotency-key-conflict"
        | "idempotency-key-reused"
        | "retitle-turn-conflict"
        | "display-revision-conflict"
        | "display-role-bound"
        | "title-mode-pinned"
        | "retitle-state-conflict"
        | "retitle-v3-state-conflict"
        | "retitle-v3-turn-conflict"
        | "retitle-v3-history-conflict"
        | "retitle-v3-history-stale"
        | "retitle-v3-idempotency-conflict"
        | "codex-account-session-busy"
        | "provider-session-already-running"
        | "history-session-live"
        | "agent-blocked"
        | "auth-quarantined"
        | "send-submit-stuck"
        | "session-has-live-children" => StatusCode::CONFLICT,
        "composer-not-empty" | "composer-unobservable" => StatusCode::CONFLICT,
        "retitle-v3-memory-not-ready" => StatusCode::UNPROCESSABLE_ENTITY,
        "retitle-v3-objective-unavailable" => StatusCode::UNPROCESSABLE_ENTITY,
        "board-cursor-expired" => StatusCode::GONE,
        "board-relay-disabled" | "console-start-disabled" => StatusCode::CONFLICT,
        "board-relay-unavailable" | "board-relay-unauthorized" | "console-start-unavailable" => {
            StatusCode::BAD_GATEWAY
        }
        "retitle-v3-history-unavailable" | "retitle-v3-history-degraded" => {
            StatusCode::SERVICE_UNAVAILABLE
        }
        // Provider account conditions share one status across providers.
        code if code.ends_with("-account-session-incarnation-conflict")
            || code.ends_with("-account-unsupported") =>
        {
            StatusCode::CONFLICT
        }
        _ => match data.exit_code {
            exit::USAGE => StatusCode::BAD_REQUEST,
            exit::DATA => StatusCode::UNPROCESSABLE_ENTITY,
            exit::UNAVAILABLE => StatusCode::SERVICE_UNAVAILABLE,
            _ => StatusCode::INTERNAL_SERVER_ERROR,
        },
    };
    let mut error = json!({ "code": data.code, "message": data.message });
    if let Some(details) = data.details
        && let Some(map) = error.as_object_mut()
    {
        map.insert("details".to_string(), details);
    }
    let mut response = (
        status,
        Json(json!({
            "schema_version": serve_schema(),
            "ok": false,
            "error": error,
        })),
    )
        .into_response();
    response.extensions_mut().insert(retained_error);
    if let Some(id) = session_id {
        response
            .extensions_mut()
            .insert(crate::lifecycle::ResponseSessionId(id));
    }
    response
}
