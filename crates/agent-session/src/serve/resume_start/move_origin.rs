//! Authenticate the exact managed caller before accessing the origin move fence.
use axum::extract::Request;
use axum::http::Method;

use super::*;
use crate::coordination::{
    console_resume_move::{
        Reply, origin,
        refusal::Failure,
        wire::{BINDING_HEADER, MAX_START_BYTES, StartRequest, StartShape, capability},
    },
    delegation,
};

pub(super) fn routes() -> Router<Arc<ServeState>> {
    Router::new().route(
        "/sessions/{caller}/console-resume-move/v1",
        get(discover).post(start),
    )
}

fn refuse(status: StatusCode, code: &'static str) -> Response {
    response(Reply::failure(Failure::before_start(status, code), None))
}

fn response(reply: Reply) -> Response {
    let (status, body) = reply.into_parts();
    (status, Json(body)).into_response()
}

fn credential(headers: &HeaderMap) -> Result<String, Response> {
    let Some(token) = super::move_routes::single_header(headers, AUTHORIZATION.as_str())
        .and_then(|value| value.strip_prefix("Bearer "))
    else {
        return Err(refuse(StatusCode::UNAUTHORIZED, "resume-move-unavailable"));
    };
    if delegation::is_delegation(token) {
        return Err(envelope_err(delegation::scope_denied()));
    }
    if headers.contains_key(BINDING_HEADER)
        || headers.contains_key(super::move_routes::RELAY_TOKEN_HEADER)
    {
        return Err(refuse(StatusCode::BAD_REQUEST, "resume-unsupported"));
    }
    Ok(token.to_owned())
}

async fn discover(
    State(state): State<Arc<ServeState>>,
    AxPath(caller): AxPath<String>,
    request: Request,
) -> Response {
    let (parts, body) = request.into_parts();
    let token = match credential(&parts.headers) {
        Ok(token) => token,
        Err(response) => return response,
    };
    if parts.method != Method::GET
        || parts.uri.query().is_some()
        || axum::body::to_bytes(body, 0).await.is_err()
    {
        return refuse(StatusCode::BAD_REQUEST, "resume-unsupported");
    }
    tokio::task::spawn_blocking(move || {
        if delegation::authenticate_read_only(
            &state.context,
            Some(&caller),
            &token,
            delegation::Use::StartViaConsole,
        )
        .is_err()
        {
            return refuse(StatusCode::UNAUTHORIZED, "resume-move-unavailable");
        }
        if state.federation.is_none() {
            return refuse(StatusCode::SERVICE_UNAVAILABLE, "resume-move-unavailable");
        }
        Json(capability(&state.machine)).into_response()
    })
    .await
    .unwrap_or_else(|_| refuse(StatusCode::SERVICE_UNAVAILABLE, "resume-move-unavailable"))
}

async fn start(
    State(state): State<Arc<ServeState>>,
    AxPath(caller): AxPath<String>,
    request: Request,
) -> Response {
    let (parts, body) = request.into_parts();
    let token = match credential(&parts.headers) {
        Ok(token) => token,
        Err(response) => return response,
    };
    let unsupported = || refuse(StatusCode::BAD_REQUEST, "resume-unsupported");
    if parts.uri.query().is_some() {
        return unsupported();
    }
    let Ok(bytes) = axum::body::to_bytes(body, MAX_START_BYTES).await else {
        return unsupported();
    };
    tokio::task::spawn_blocking(move || {
        let Ok((_, incarnation)) = delegation::authenticate_token(
            &state.context,
            Some(&caller),
            &token,
            delegation::Use::StartViaConsole,
            true,
        ) else {
            return refuse(StatusCode::UNAUTHORIZED, "resume-move-unavailable");
        };
        let Ok(body) = std::str::from_utf8(&bytes) else {
            return unsupported();
        };
        let mismatch = || refuse(StatusCode::FORBIDDEN, "resume-move-binding-mismatch");
        let request = match StartRequest::parse(body.as_bytes()) {
            Ok(request) => request,
            Err(StartShape::Unsupported) => return unsupported(),
            Err(StartShape::OwnerAssertion) => return mismatch(),
        };
        if !request.matches_caller(&caller, &incarnation) {
            return mismatch();
        }
        response(origin::start(
            &state.context,
            state.federation.as_ref(),
            &request,
            body,
        ))
    })
    .await
    .unwrap_or_else(|_| response(Reply::failure(Failure::unknown(None), None)))
}
