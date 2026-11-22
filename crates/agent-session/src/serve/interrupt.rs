use super::*;
use crate::codex_app_server::interrupt::{Error, MINIMUM_CODEX_VERSION};

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Body {
    expected_session_incarnation: String,
}

pub(super) async fn handler(
    State(state): State<Arc<ServeState>>,
    headers: HeaderMap,
    AxPath(id): AxPath<String>,
    body: Result<Json<Body>, JsonRejection>,
) -> Response {
    if let Some(response) = deny_unauthorized(&state, &headers) {
        return response;
    }
    let Ok(Json(body)) = body else {
        return status_json(
            StatusCode::BAD_REQUEST,
            "invalid-json-body",
            "request body must contain expected_session_incarnation",
        );
    };
    let launch_id = &body.expected_session_incarnation;
    if !prompt_receipts::valid_incarnation(launch_id) {
        return status_json(
            StatusCode::BAD_REQUEST,
            "invalid-session-incarnation",
            "expected_session_incarnation must be a nonblank identifier of at most 128 bytes",
        );
    }
    let record = match load_structured_prompt_record_locked(&state, &id, Some(launch_id)).await {
        Ok(record) => record,
        Err(response) => return response,
    };
    if !codex_app_server::runtime_is_supported(&record) {
        return refusal(Error::Unsupported);
    }
    let Some(handle) = wait_for_codex_control(&state, &record.id, launch_id).await else {
        return refusal(Error::Unavailable);
    };
    let Ok(thread_id) = codex_app_server::guarded::Client::attached_thread(&record) else {
        return refusal(Error::Unavailable);
    };
    match handle.interrupt(thread_id).await {
        Ok(outcome) => envelope_ok(json!({
            "machine": state.machine,
            "session_id": record.id,
            "session_incarnation": launch_id,
            "outcome": outcome,
        })),
        Err(error) => refusal(error),
    }
}

fn refusal(error: Error) -> Response {
    match error {
        Error::Busy => status_json(
            StatusCode::CONFLICT,
            "codex-interrupt-busy",
            "another Codex turn start or session mutation is in flight",
        ),
        Error::TargetChanged => status_json(
            StatusCode::CONFLICT,
            "codex-interrupt-target-changed",
            "the managed Codex runtime or bound thread changed before interruption",
        ),
        Error::Unsupported => status_json(
            StatusCode::CONFLICT,
            "codex-interrupt-unsupported",
            &format!(
                "structured interruption requires a managed app-server and Codex {MINIMUM_CODEX_VERSION} or later"
            ),
        ),
        Error::Unavailable => status_json(
            StatusCode::SERVICE_UNAVAILABLE,
            "codex-interrupt-unavailable",
            &format!(
                "Codex interrupt control or turn state is unavailable; the managed path requires Codex {MINIMUM_CODEX_VERSION} or later"
            ),
        ),
        Error::OutcomeUnknown => status_json(
            StatusCode::BAD_GATEWAY,
            "codex-interrupt-outcome-unknown",
            "Codex did not confirm the targeted turn's terminal status",
        ),
    }
}
