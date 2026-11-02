//! Authenticated saved-draft inventory and byte-exact read access.
use super::*;
use crate::send_guard::drafts;

pub(super) fn routes() -> Router<Arc<ServeState>> {
    Router::new()
        .route("/sessions/{id}/drafts", get(list))
        .route("/sessions/{id}/drafts/{draft_id}", get(read))
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
    match tokio::task::spawn_blocking(move || {
        let _lock = crate::acquire_session_record_lock(&context, &id)?;
        let record = load_session_record(&context, &id)?;
        drafts::list(&context, &record)
            .map(|drafts| drafts.into_iter().map(|d| d.metadata()).collect::<Vec<_>>())
    })
    .await
    {
        Ok(Ok(drafts)) => envelope_ok(json!({"machine":state.machine,"drafts":drafts})),
        Ok(Err(error)) => envelope_err(error),
        Err(_) => join_err(),
    }
}

async fn read(
    State(state): State<Arc<ServeState>>,
    headers: HeaderMap,
    AxPath((id, draft_id)): AxPath<(String, String)>,
) -> Response {
    if let Some(response) = deny_unauthorized(&state, &headers) {
        return response;
    }
    let context = state.context.clone();
    match tokio::task::spawn_blocking(move || {
        let _lock = crate::acquire_session_record_lock(&context, &id)?;
        let record = load_session_record(&context, &id)?;
        drafts::read(&context, &record, &draft_id).map(|d| d.text())
    })
    .await
    {
        Ok(Ok(text)) => (
            [
                ("content-type", "text/plain; charset=utf-8"),
                ("cache-control", "no-store"),
            ],
            text,
        )
            .into_response(),
        Ok(Err(error)) => envelope_err(error),
        Err(_) => join_err(),
    }
}
