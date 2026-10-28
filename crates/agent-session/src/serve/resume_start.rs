//! Prompt-free preflight and a versioned create endpoint that older daemons cannot misread.
use super::*;
use crate::start_resume::{PREFLIGHT_SCHEMA, Request, refusal};

pub(super) fn deserialize_account_consent<'de, D>(deserializer: D) -> Result<Option<bool>, D::Error>
where
    D: serde::Deserializer<'de>,
{
    <bool as serde::Deserialize>::deserialize(deserializer).map(Some)
}

pub(super) fn routes() -> Router<Arc<ServeState>> {
    Router::new()
        .route("/resume-preflight/v1", post(target_check))
        .route("/sessions/{id}/resume-preflight/v1", post(relay_check))
        .route("/sessions/resume/v1", post(create))
        .route("/sessions/{id}/console-resume/v1", post(console_create))
}

fn response(result: Result<Value, CliError>) -> Response {
    match result {
        Ok(data) => {
            Json(json!({"schema_version":PREFLIGHT_SCHEMA,"ok":true,"data":data})).into_response()
        }
        Err(error) => {
            let mut response = envelope_err(error.clone());
            let inner = error.into_inner();
            *response.body_mut() = Body::from(
                json!({"schema_version":PREFLIGHT_SCHEMA,"ok":false,
                "error":{"code":inner.code,"message":inner.message,"details":inner.details}})
                .to_string(),
            );
            response
        }
    }
}

async fn target_check(
    State(state): State<Arc<ServeState>>,
    headers: HeaderMap,
    body: Result<Json<Value>, JsonRejection>,
) -> Response {
    if let Some(response) = deny_unauthorized(&state, &headers) {
        return response;
    }
    let request: Request = match body
        .ok()
        .and_then(|Json(body)| serde_json::from_value(body).ok())
    {
        Some(request) => request,
        None => return response(Err(refusal("resume-unsupported"))),
    };
    match tokio::task::spawn_blocking(move || {
        crate::start_resume::preflight(&state.context, &request, &state.tmux_bin, &state.machine)
    })
    .await
    {
        Ok(result) => response(result),
        Err(_) => join_err(),
    }
}

async fn relay_check(
    State(state): State<Arc<ServeState>>,
    headers: HeaderMap,
    AxPath(id): AxPath<String>,
    body: Result<Json<Value>, JsonRejection>,
) -> Response {
    let token = match remote_session_token(&headers) {
        Ok(token) => token,
        Err(error) => return response(Err(error)),
    };
    let Ok(Json(body)) = body else {
        return response(Err(refusal("resume-unsupported")));
    };
    match tokio::task::spawn_blocking(move || {
        crate::coordination::resume_preflight::relay_route(
            &state.context,
            state.federation.as_ref(),
            &id,
            &token,
            &body,
        )
    })
    .await
    {
        Ok(result) => response(result),
        Err(_) => join_err(),
    }
}

async fn create(
    State(state): State<Arc<ServeState>>,
    headers: HeaderMap,
    body: Result<Json<CreateBody>, JsonRejection>,
) -> Response {
    if let Some(response) = deny_unauthorized(&state, &headers) {
        return response;
    }
    let Ok(Json(body)) = body else {
        return envelope_err(refusal("resume-unsupported"));
    };
    if body.resume.is_none()
        || body.provider_resume_id.is_some()
        || body.agent_profile.is_some()
        || body.codex_account.is_some()
    {
        return envelope_err(refusal("resume-unsupported"));
    }
    let request = Request {
        agent: body.agent.clone(),
        resume: body.resume.clone().unwrap(),
        cwd: body.cwd.as_ref().map(PathBuf::from),
        account: body.claude_account.clone(),
        agent_profile: body.agent_profile.clone(),
        agent_args: body.agent_args.clone(),
    };
    if let Err(error) = request.validate() {
        return envelope_err(error);
    }
    if let Err(error) = crate::launch_env::validate(&body.launch_env) {
        return envelope_err(error);
    }
    let origin = match body.origin_fields() {
        Ok(origin) => origin,
        Err(error) => return envelope_err(error),
    };
    // start_resume owns the journal only after guarded final admission. In particular,
    // do not run the ordinary HTTP wrapper that records pre-engine create failures.
    super::create::create_handler_inner(State(state), headers, Json(body), origin).await
}

/// A capability-specific path lets a CLI replay reach the existing receipt
/// before admission, without an older origin interpreting resume as fresh start.
async fn console_create(
    State(state): State<Arc<ServeState>>,
    headers: HeaderMap,
    AxPath(id): AxPath<String>,
    body: Result<Json<Value>, JsonRejection>,
) -> Response {
    let Ok(Json(body)) = body else {
        return envelope_err(refusal("resume-unsupported"));
    };
    if body["session"]["resume"].as_str().is_none() && body["reconcile"] != true {
        return envelope_err(refusal("resume-unsupported"));
    }
    super::console_start_handler(State(state), headers, AxPath(id), Ok(Json(body))).await
}
