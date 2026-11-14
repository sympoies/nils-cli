//! Operator-authenticated explicit Claude hook decisions; mailbox auth is insufficient.
use super::*;
use crate::cli::permission::{self, AnswerRequest};

pub(super) fn routes() -> Router<Arc<ServeState>> {
    Router::new().route("/sessions/{id}/permissions", get(list).post(answer))
}
async fn list(
    State(state): State<Arc<ServeState>>,
    headers: HeaderMap,
    AxPath(id): AxPath<String>,
) -> Response {
    if let Some(response) = deny_unauthorized(&state, &headers) {
        return response;
    }
    let context = state.context.clone();
    match tokio::task::spawn_blocking(move || permission::list(&context, &id)).await {
        Ok(Ok(value)) => envelope_ok(value),
        Ok(Err(error)) => envelope_err(error),
        Err(_) => join_err(),
    }
}
async fn answer(
    State(state): State<Arc<ServeState>>,
    headers: HeaderMap,
    AxPath(id): AxPath<String>,
    body: Result<Json<AnswerRequest>, JsonRejection>,
) -> Response {
    if let Some(response) = deny_unauthorized(&state, &headers) {
        return response;
    }
    let Json(request) = match body {
        Ok(request) => request,
        Err(_) => {
            return envelope_err(CliError::usage(
                "invalid-json-body",
                "permission answer requires request_id, call_id and allow or deny decision",
                None,
            ));
        }
    };
    let context = state.context.clone();
    match tokio::task::spawn_blocking(move || permission::answer(&context, &id, request)).await {
        Ok(Ok(value)) => envelope_ok(value),
        Ok(Err(error)) => envelope_err(error),
        Err(_) => join_err(),
    }
}
