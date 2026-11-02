use super::*;

#[derive(Debug, Deserialize)]
struct HistoryMessagesQuery {
    cursor: Option<String>,
    limit: Option<usize>,
    direction: Option<HistoryMessageDirection>,
    parts: Option<HistoryPartsVersion>,
}

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
    let include_parts = query.parts == Some(HistoryPartsVersion::V1);
    match tokio::task::spawn_blocking(move || {
        catalog.messages_with_mailbox_events(
            &id,
            query.cursor.as_deref(),
            query.limit.unwrap_or(50),
            query.direction.unwrap_or_default(),
            include_parts,
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
        Ok(Ok(image)) => {
            let filename = image.filename.as_deref().and_then(crate::attachment_filename::content_disposition);
            let mut response = ([(CONTENT_TYPE, image.mime)], image.bytes).into_response();
            if let Some(filename) = filename {
                response.headers_mut().insert("content-disposition", filename);
            }
            response
        },
        Ok(Err(provider_history::history_images::ImageReadError::Deleted)) => (
            StatusCode::NOT_FOUND, Json(json!({"ok":false,"error":{"code":"history-image-deleted","message":"Image deleted"}}))
        ).into_response(),
        Ok(Err(provider_history::history_images::ImageReadError::Unavailable)) => (
            StatusCode::UNPROCESSABLE_ENTITY, Json(json!({"ok":false,"error":{"code":"history-image-unavailable","message":"Image unavailable"}}))
        ).into_response(),
        Ok(Err(provider_history::history_images::ImageReadError::OutsideAllowedRoots)) => (
            StatusCode::UNPROCESSABLE_ENTITY, Json(json!({"ok":false,"error":{
                "code":"history-image-unavailable",
                "reason":"outside-allowed-roots",
                "message":"Image is outside the recorded working directory and owned attachments"
            }}))
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
#[derive(Clone, Copy, Debug, Deserialize, PartialEq, Eq)]
pub(super) enum HistoryPartsVersion {
    #[serde(rename = "v1")]
    V1,
}

#[cfg(test)]
#[path = "uploaded_image_tests.rs"]
mod uploaded_image_tests;

#[cfg(test)]
#[path = "history_image_regression_tests.rs"]
mod history_image_regression_tests;

#[cfg(test)]
#[path = "mailbox_read_events_route_tests.rs"]
mod mailbox_read_events_route_tests;
