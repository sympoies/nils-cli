//! Routes that read or refresh one session's displayed facts.
use super::*;
mod drafts;
mod permission;

pub(super) fn routes() -> Router<Arc<ServeState>> {
    Router::new()
        .merge(drafts::routes())
        .merge(permission::routes())
        .route("/sessions/{id}/retitle", post(session_retitle_handler))
        .route("/sessions/{id}/refresh", post(session_refresh_handler))
        .route("/sessions/{id}/metrics/v1", get(session_metrics_handler))
}

async fn session_retitle_handler(
    State(state): State<Arc<ServeState>>,
    headers: HeaderMap,
    AxPath(id): AxPath<String>,
    body: Result<Json<crate::retitle::RetitleRequest>, JsonRejection>,
) -> Response {
    if let Some(response) = deny_unauthorized(&state, &headers) {
        return response;
    }
    let Json(request) = match body {
        Ok(body) => body,
        Err(_) => {
            return envelope_err(CliError::usage(
                "invalid-json-body",
                "retitle request body is invalid",
                None,
            ));
        }
    };
    match execute_retitle(state.clone(), id, request).await {
        Ok(retitle) => envelope_ok(json!({
            "machine": state.machine,
            "retitle": retitle,
        })),
        Err(error) => envelope_err(error),
    }
}

async fn session_refresh_handler(
    State(state): State<Arc<ServeState>>,
    headers: HeaderMap,
    AxPath(id): AxPath<String>,
) -> Response {
    if let Some(response) = deny_unauthorized(&state, &headers) {
        return response;
    }
    let context = state.context.clone();
    match tokio::task::spawn_blocking(move || crate::session_model::refresh::refresh(&context, &id))
        .await
    {
        Ok(Ok(refresh)) => envelope_ok(json!({
            "machine": state.machine,
            "refresh": refresh,
        })),
        Ok(Err(error)) => envelope_err(error),
        Err(_) => join_err(),
    }
}

async fn session_metrics_handler(
    State(state): State<Arc<ServeState>>,
    headers: HeaderMap,
    AxPath(id): AxPath<String>,
) -> Response {
    if let Some(response) = deny_unauthorized(&state, &headers) {
        return response;
    }
    let context = state.context.clone();
    match tokio::task::spawn_blocking(move || {
        let record = crate::load_session_record(&context, &id)?;
        Ok::<_, CliError>(crate::context_usage::session_metrics::collect(
            &context, &record,
        ))
    })
    .await
    {
        Ok(Ok(metrics)) => envelope_ok(json!({"machine": state.machine, "metrics": metrics})),
        Ok(Err(error)) => envelope_err(error),
        Err(_) => join_err(),
    }
}

#[cfg(test)]
#[path = "session_metrics_tests.rs"]
mod session_metrics_tests;
