//! Minimal local HTTP front end; no provisioning, auth or service platform.
use crate::{Engine, Operation, Request, ResearchResult};
use axum::{
    Json, Router,
    body::Bytes,
    extract::{DefaultBodyLimit, State},
    http::StatusCode,
    response::IntoResponse,
    routing::post,
};
use nils_common::cli_contract::{Envelope, EnvelopeError};

pub fn router(engine: Engine) -> Router {
    Router::new()
        .route("/v1/search", post(search))
        .route("/v1/read-url", post(read_url))
        .route("/v1/brief", post(brief))
        .fallback(|| async {
            failure(
                "invalid-request",
                "Unknown research endpoint",
                StatusCode::NOT_FOUND,
            )
        })
        .method_not_allowed_fallback(|| async {
            failure(
                "invalid-request",
                "Use POST for research operations",
                StatusCode::METHOD_NOT_ALLOWED,
            )
        })
        .layer(DefaultBodyLimit::max(65_536))
        .with_state(engine)
}
async fn search(
    State(engine): State<Engine>,
    body: Result<Bytes, axum::extract::rejection::BytesRejection>,
) -> impl IntoResponse {
    handle(engine, Operation::Search, body).await
}
async fn read_url(
    State(engine): State<Engine>,
    body: Result<Bytes, axum::extract::rejection::BytesRejection>,
) -> impl IntoResponse {
    handle(engine, Operation::ReadUrl, body).await
}
async fn brief(
    State(engine): State<Engine>,
    body: Result<Bytes, axum::extract::rejection::BytesRejection>,
) -> impl IntoResponse {
    handle(engine, Operation::Brief, body).await
}
async fn handle(
    engine: Engine,
    operation: Operation,
    body: Result<Bytes, axum::extract::rejection::BytesRejection>,
) -> (StatusCode, Json<Envelope<ResearchResult>>) {
    let Ok(body) = body else {
        return failure(
            "invalid-request",
            "HTTP request body limit exceeded",
            StatusCode::BAD_REQUEST,
        );
    };
    let request: Request = match serde_json::from_slice(&body) {
        Ok(request) => request,
        Err(_) => {
            return failure(
                "invalid-request",
                "Malformed research request JSON",
                StatusCode::BAD_REQUEST,
            );
        }
    };
    if request.operation != operation {
        return failure(
            "invalid-request",
            "Request operation must match HTTP route",
            StatusCode::BAD_REQUEST,
        );
    }
    let envelope = engine.execute(request).await;
    let status = if envelope.ok {
        StatusCode::OK
    } else {
        match envelope.error.as_ref().map(|e| e.code.as_str()) {
            Some("invalid-request" | "invalid-url" | "non-public-url") => StatusCode::BAD_REQUEST,
            Some("rate-limited") => StatusCode::TOO_MANY_REQUESTS,
            Some("timeout") => StatusCode::GATEWAY_TIMEOUT,
            _ => StatusCode::BAD_GATEWAY,
        }
    };
    (status, Json(envelope))
}
fn failure(
    code: &str,
    message: &str,
    status: StatusCode,
) -> (StatusCode, Json<Envelope<ResearchResult>>) {
    (
        status,
        Json(Envelope::failure(
            "cli.research-cli.error.v1",
            EnvelopeError::new(code, message),
        )),
    )
}
