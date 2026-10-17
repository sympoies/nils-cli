use super::*;

pub(super) fn routes() -> Router<Arc<ServeState>> {
    Router::new()
        .route(
            "/history/sessions/{id}/messages",
            get(history_messages_handler),
        )
        .route(
            "/history/sessions/{id}/images/{image_id}",
            get(image_handler),
        )
}

async fn history_messages_handler(
    State(state): State<Arc<ServeState>>,
    headers: HeaderMap,
    AxPath(id): AxPath<String>,
    Query(query): Query<HistoryMessagesQuery>,
) -> Response {
    if let Some(response) = deny_unauthorized(&state, &headers) {
        return response;
    }
    let catalog = state.history_catalog.clone();
    match tokio::task::spawn_blocking(move || {
        catalog.messages(
            &id,
            query.cursor.as_deref(),
            query.limit.unwrap_or(50),
            query.direction.unwrap_or_default(),
        )
    })
    .await
    {
        Ok(Ok(page)) => envelope_ok(json!({
            "machine": state.machine,
            "messages": page.messages,
            "next_cursor": page.next_cursor,
            "older_cursor": page.older_cursor,
        })),
        Ok(Err(error)) => history_error_response(error),
        Err(_) => join_err(),
    }
}

async fn image_handler(
    State(state): State<Arc<ServeState>>,
    headers: HeaderMap,
    AxPath((id, image_id)): AxPath<(String, String)>,
) -> Response {
    if let Some(response) = deny_unauthorized(&state, &headers) {
        return response;
    }
    let catalog = state.history_catalog.clone();
    let mut response = match tokio::task::spawn_blocking(move || catalog.image(&id, &image_id)).await {
        Ok(Ok(image)) => ([(CONTENT_TYPE, image.mime)], image.bytes).into_response(),
        Ok(Err(provider_history::history_images::ImageReadError::Deleted)) => (
            StatusCode::NOT_FOUND, Json(json!({"ok":false,"error":{"code":"history-image-deleted","message":"Image deleted"}}))
        ).into_response(),
        Ok(Err(provider_history::history_images::ImageReadError::Unavailable)) => (
            StatusCode::UNPROCESSABLE_ENTITY, Json(json!({"ok":false,"error":{"code":"history-image-unavailable","message":"Image unavailable"}}))
        ).into_response(),
        Err(_) => join_err(),
    };
    response
        .headers_mut()
        .insert("cache-control", "private, no-store".parse().unwrap());
    response
        .headers_mut()
        .insert("x-content-type-options", "nosniff".parse().unwrap());
    response
}

#[cfg(test)]
#[path = "history_images_tests.rs"]
mod tests;
