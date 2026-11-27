//! Authenticated relay ingress for the destination move routes.
use axum::extract::{FromRequestParts, Request};
use axum::http::{HeaderValue, Method, header::CACHE_CONTROL};

use super::*;
use crate::cli::resume_move::{canonical_uuid, transport};
use crate::coordination::console_resume_move::{
    refusal::Failure,
    wire::{
        BINDING_HEADER, Binding, MAX_START_BYTES, START_SCHEMA, STATUS_SCHEMA, StartRequest,
        StartShape,
    },
};

pub(super) const RELAY_TOKEN_HEADER: &str = "x-agent-session-relay-token";

pub(super) fn routes() -> Router<Arc<ServeState>> {
    Router::new()
        .route("/sessions/resume-move/v1", post(destination_start))
        .route(
            "/sessions/resume-move/{move_id}/status/v1",
            get(destination_status),
        )
}

pub(in crate::serve) fn advertised(state: &ServeState) -> bool {
    state.federation.is_some() && transport::configured()
}

pub(in crate::serve) fn has_machine_bearer(state: &ServeState, headers: &HeaderMap) -> bool {
    state.token.as_deref().is_some_and(|expected| {
        single_header(headers, AUTHORIZATION.as_str())
            .and_then(|value| value.strip_prefix("Bearer "))
            .is_some_and(|provided| constant_time_eq(provided, expected))
    })
}

fn authorize(state: &ServeState, headers: &HeaderMap) -> Result<(), StatusCode> {
    let Some(federation) = state.federation.as_ref().filter(|_| state.token.is_some()) else {
        return Err(StatusCode::SERVICE_UNAVAILABLE);
    };
    let valid_relay = single_header(headers, RELAY_TOKEN_HEADER)
        .is_some_and(|token| constant_time_eq(token, &federation.ingress_token));
    if !has_machine_bearer(state, headers) || !valid_relay {
        return Err(StatusCode::UNAUTHORIZED);
    }
    Ok(())
}

pub(super) fn single_header<'h>(headers: &'h HeaderMap, name: &str) -> Option<&'h str> {
    let values = headers.get_all(name);
    let mut values = values.iter();
    let value = values.next()?.to_str().ok()?;
    values.next().is_none().then_some(value)
}

fn binding(headers: &HeaderMap) -> Option<Binding> {
    Binding::decode(single_header(headers, BINDING_HEADER)?)
}

async fn destination_start(State(state): State<Arc<ServeState>>, request: Request) -> Response {
    let (parts, body) = request.into_parts();
    let refuse = |status, code| failure(START_SCHEMA, None, Failure::before_start(status, code));
    if let Err(status) = authorize(&state, &parts.headers) {
        return refuse(status, "resume-move-unavailable");
    }
    let shape_error = || refuse(StatusCode::BAD_REQUEST, "resume-unsupported");
    if parts.uri.query().is_some() {
        return shape_error();
    }
    let Ok(bytes) = axum::body::to_bytes(body, MAX_START_BYTES).await else {
        return shape_error();
    };
    let bad_binding = || refuse(StatusCode::FORBIDDEN, "resume-move-binding-mismatch");
    let request = match StartRequest::parse(&bytes) {
        Ok(request) => request,
        Err(StartShape::Unsupported) => return shape_error(),
        Err(StartShape::OwnerAssertion) => return bad_binding(),
    };
    if !binding(&parts.headers)
        .is_some_and(|binding| binding.matches_start(&state.machine, &request))
    {
        return bad_binding();
    }
    let move_id = request.session.move_id.clone();
    let result = tokio::task::spawn_blocking(move || {
        transport::start_wire(
            &state.context,
            &state.tmux_bin,
            &state.machine,
            request.session,
        )
    })
    .await
    .unwrap_or_else(|_| Err(Failure::unknown(None)));
    reply(START_SCHEMA, &move_id, result)
}

async fn destination_status(State(state): State<Arc<ServeState>>, request: Request) -> Response {
    let mut response = read_status(state, request).await;
    response
        .headers_mut()
        .insert(CACHE_CONTROL, HeaderValue::from_static("no-store"));
    response
}

async fn read_status(state: Arc<ServeState>, request: Request) -> Response {
    let (mut parts, body) = request.into_parts();
    let refuse = |status, code| failure(STATUS_SCHEMA, None, Failure::read(status, code));
    if let Err(status) = authorize(&state, &parts.headers) {
        return refuse(status, "resume-move-unavailable");
    }
    let shape_error = || refuse(StatusCode::BAD_REQUEST, "resume-unsupported");
    let Ok(AxPath(move_id)) = AxPath::<String>::from_request_parts(&mut parts, &state).await else {
        return shape_error();
    };
    if parts.method != Method::GET
        || parts.uri.query().is_some()
        || !canonical_uuid(&move_id)
        || axum::body::to_bytes(body, 0).await.is_err()
    {
        return shape_error();
    }
    if !binding(&parts.headers)
        .is_some_and(|binding| binding.matches_read(&state.machine, &move_id))
    {
        return refuse(StatusCode::FORBIDDEN, "resume-move-binding-mismatch");
    }
    let id = move_id.clone();
    let result = tokio::task::spawn_blocking(move || {
        transport::status_projection(&state.context, &state.machine, &id)
    })
    .await
    .unwrap_or_else(|_| {
        Err(Failure::read(
            StatusCode::SERVICE_UNAVAILABLE,
            "resume-move-state-unavailable",
        ))
    });
    reply(STATUS_SCHEMA, &move_id, result)
}

fn reply(schema: &str, move_id: &str, result: Result<Value, Failure>) -> Response {
    match result {
        Ok(data) => Json(json!({"schema_version":schema,"ok":true,"data":data})).into_response(),
        Err(error) => failure(schema, Some(move_id), error),
    }
}

fn failure(schema: &str, move_id: Option<&str>, error: Failure) -> Response {
    let (status, body) = error.envelope(schema, move_id);
    (status, Json(body)).into_response()
}
