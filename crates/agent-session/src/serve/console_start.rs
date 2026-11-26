use super::*;

/// `POST /sessions/{id}/console-start/v1`: start or reconcile an owned child.
pub(super) async fn handler(
    State(state): State<Arc<ServeState>>,
    headers: HeaderMap,
    AxPath(id): AxPath<String>,
    body: Result<Json<strict_json::Unique>, JsonRejection>,
) -> Response {
    let token = match remote_session_token(&headers) {
        Ok(v) => v,
        Err(e) => return envelope_err(e),
    };
    let body = match body {
        Ok(Json(strict_json::Unique(body))) => body,
        Err(JsonRejection::JsonDataError(_)) => {
            return envelope_err(crate::coordination::console_start::attestation::invalid_body());
        }
        Err(_) => {
            return status_json(
                StatusCode::BAD_REQUEST,
                "console-start-invalid",
                "the console start body must be a JSON object",
            );
        }
    };
    let context = state.context.clone();
    let federation = state.federation.clone();
    match tokio::task::spawn_blocking(move || {
        crate::coordination::console_start::relay_route(
            &context,
            federation.as_ref(),
            &id,
            &token,
            &body,
        )
    })
    .await
    {
        Ok(Ok(value)) => (StatusCode::CREATED, Json(value)).into_response(),
        Ok(Err(err)) => envelope_err(err),
        Err(_) => join_err(),
    }
}
