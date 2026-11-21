//! Session-capability ingress for the companion producer.

use axum::extract::{DefaultBodyLimit, FromRequest, Request};

use super::*;
use crate::coordination::companion::{self, Failure, Submission};

pub(super) fn routes() -> Router<Arc<ServeState>> {
    Router::new()
        .route(companion::ROUTE, post(submit))
        .layer(DefaultBodyLimit::max(companion::MAX_REQUEST_BYTES))
}

async fn submit(State(state): State<Arc<ServeState>>, request: Request) -> Response {
    let (parts, body) = request.into_parts();
    let token = match remote_session_token(&parts.headers) {
        Ok(token) if crate::activity_ingress::from_loopback_peer(&parts) => token,
        _ => return envelope_err(Failure::Forbidden.into()),
    };
    if parts.uri.query().is_some() {
        return envelope_err(Failure::Invalid.into());
    }

    let request = Request::from_parts(parts, body);
    let submission = match tokio::time::timeout(
        Duration::from_secs(5),
        Json::<Submission>::from_request(request, &()),
    )
    .await
    {
        Ok(Ok(Json(submission))) => submission,
        Ok(Err(error)) if error.status() == StatusCode::PAYLOAD_TOO_LARGE => {
            return envelope_err(Failure::TooLarge.into());
        }
        _ => return envelope_err(Failure::Invalid.into()),
    };
    match tokio::task::spawn_blocking(move || {
        companion::submit_relay(
            &state.context,
            state.federation.as_ref(),
            &token,
            submission,
        )
    })
    .await
    {
        Ok(Ok(receipt)) => envelope_status(StatusCode::CREATED, json!(receipt)),
        Ok(Err(failure)) => envelope_err(failure.into()),
        Err(_) => envelope_err(Failure::Ambiguous.into()),
    }
}
