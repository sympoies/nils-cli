//! Admission for relay-attested fresh Console roots.
use super::*;
use axum::http::header::AsHeaderName;
use base64::{Engine, engine::general_purpose::URL_SAFE_NO_PAD};
use nils_common::forge_identity::session::{CONTEXT_KEY, LaunchContext};

mod strict_json;

const MEDIA_TYPE: &str = "application/vnd.agent-session.console-attested-create.v1";
const TOKEN_HEADER: &str = "x-agent-session-attested-create-token";
const BINDING_HEADER: &str = "x-agent-session-console-create-binding";
const BINDING_SCHEMA: &str = "agent-session.console-create-binding.v1";
const MAX_BINDING_BYTES: usize = 2048;
const ROLE: &str = "steward";

/// Only constructed after both credentials, the binding and the body pass.
pub(in crate::serve) struct Verified(());

impl Verified {
    pub(super) fn set_lineage(self, machine: &str, lineage: &mut crate::InitialLineage) {
        let mut seed = crate::lineage::LineageSeed::root(
            machine,
            crate::lineage::STARTER_CONSOLE,
            crate::lineage::VIA_CONSOLE,
        );
        seed.set_forge_context(Some(LaunchContext {
            initiator: crate::forge_identity::CONSOLE_RELAY_INITIATOR.into(),
            role: Some(ROLE.into()),
            attested_role: Some(ROLE.into()),
        }));
        lineage.seed = seed;
        lineage.role = Some(ROLE.into());
    }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Binding {
    schema_version: String,
    scope: String,
    origin_machine: String,
    caller_session_id: String,
    caller_session_incarnation: String,
    destination_machine: String,
    attested_role: String,
    body_sha256: String,
}

impl Binding {
    fn read(headers: &HeaderMap, machine: &str) -> Option<Self> {
        let raw = single_header(headers, BINDING_HEADER)?;
        if raw.len() > MAX_BINDING_BYTES {
            return None;
        }
        let decoded = URL_SAFE_NO_PAD.decode(raw).ok()?;
        if decoded.iter().find(|byte| !byte.is_ascii_whitespace()) != Some(&b'{') {
            return None;
        }
        let binding: Self = serde_json::from_slice(&decoded).ok()?;
        let incarnation = uuid::Uuid::parse_str(&binding.caller_session_incarnation).ok()?;
        (binding.schema_version == BINDING_SCHEMA
            && binding.scope == "create"
            && binding.destination_machine == machine
            && !binding.origin_machine.trim().is_empty()
            && crate::validate_id(&binding.caller_session_id).is_ok()
            && !incarnation.is_nil()
            && incarnation.hyphenated().to_string() == binding.caller_session_incarnation
            && binding.body_sha256.len() == 64
            && binding
                .body_sha256
                .bytes()
                .all(|byte| byte.is_ascii_digit() || matches!(byte, b'a'..=b'f')))
        .then_some(binding)
    }
}

pub(super) fn selected(headers: &HeaderMap) -> bool {
    headers.contains_key(TOKEN_HEADER)
        || headers.contains_key(BINDING_HEADER)
        || headers
            .get_all(CONTENT_TYPE)
            .iter()
            .filter_map(|value| value.to_str().ok())
            .any(|value| {
                value
                    .split(';')
                    .next()
                    .is_some_and(|media| media.trim().eq_ignore_ascii_case(MEDIA_TYPE))
            })
}

#[allow(clippy::result_large_err)]
pub(super) async fn admit(
    state: &ServeState,
    headers: &HeaderMap,
    request: Request,
) -> Result<(CreateBody, Verified), Response> {
    if let Some(response) = deny_unauthorized(state, headers) {
        return Err(response);
    }
    if single_header(headers, AUTHORIZATION).is_none() {
        return Err(unauthorized("missing or invalid bearer token"));
    }
    let Some(expected) = state.attested_create_token.as_deref() else {
        return Err(status_json(
            StatusCode::SERVICE_UNAVAILABLE,
            "console-create-attestation-unavailable",
            "attested create is not configured",
        ));
    };
    if !single_header(headers, TOKEN_HEADER).is_some_and(|token| constant_time_eq(token, expected))
    {
        return Err(unauthorized("missing or invalid attested create token"));
    }
    if single_header(headers, CONTENT_TYPE) != Some(MEDIA_TYPE) {
        return Err(StatusCode::UNSUPPORTED_MEDIA_TYPE.into_response());
    }
    let binding = Binding::read(headers, &state.machine).ok_or_else(binding_mismatch)?;
    let bytes = Bytes::from_request(request, &())
        .await
        .map_err(IntoResponse::into_response)?;
    if binding.body_sha256 != crate::coordination::digest_bytes(&bytes) {
        return Err(binding_mismatch());
    }
    if binding.attested_role != ROLE {
        return Err(status_json(
            StatusCode::FORBIDDEN,
            "console-create-role-forbidden",
            "attested create role is forbidden",
        ));
    }
    let body = create_body(&bytes).map_err(envelope_err)?;
    Ok((body, Verified(())))
}

fn create_body(bytes: &[u8]) -> Result<CreateBody, CliError> {
    let strict_json::Unique(value) = serde_json::from_slice(bytes).map_err(|_| invalid_body())?;
    let object = value.as_object().ok_or_else(invalid_body)?;
    let id = object
        .get("id")
        .and_then(Value::as_str)
        .ok_or_else(invalid_body)?;
    crate::validate_id(id).map_err(|_| invalid_body())?;
    let lineage = object.get("lineage").filter(|value| !value.is_null());
    let lineage_object = lineage.and_then(Value::as_object);
    if object.contains_key("role")
        || [CONTEXT_KEY, "initiator", "attested_role"]
            .iter()
            .any(|key| {
                object.contains_key(*key)
                    || lineage_object.is_some_and(|lineage| lineage.contains_key(*key))
            })
    {
        return Err(CliError::usage(
            "identity_session_binding_untrusted",
            "forge launch context requires a verified launch owner",
            None,
        ));
    }
    if lineage.is_some() && !lineage_object.is_some_and(serde_json::Map::is_empty) {
        return Err(CliError::usage(
            "role-requires-root",
            "attested create requires daemon-owned root lineage",
            None,
        ));
    }
    if [
        "resume",
        "resume_account_explicit",
        "provider_resume_id",
        "resume_id",
    ]
    .iter()
    .any(|key| object.get(*key).is_some_and(|value| !value.is_null()))
    {
        return Err(CliError::usage(
            "resume-unsupported",
            "this resume request is not supported",
            None,
        ));
    }
    let create_body::CheckedCreateBody(mut body) =
        serde_json::from_value(value).map_err(|_| invalid_body())?;
    body.lineage = None;
    Ok(body)
}

fn single_header(headers: &HeaderMap, name: impl AsHeaderName) -> Option<&str> {
    let mut values = headers.get_all(name).iter();
    let value = values.next()?;
    if values.next().is_some() {
        return None;
    }
    value.to_str().ok()
}

fn unauthorized(message: &str) -> Response {
    status_json(StatusCode::UNAUTHORIZED, "unauthorized", message)
}

fn binding_mismatch() -> Response {
    status_json(
        StatusCode::FORBIDDEN,
        "console-create-binding-mismatch",
        "invalid attested create binding",
    )
}

fn invalid_body() -> CliError {
    CliError::usage(
        "console-create-invalid-body",
        "invalid attested create request",
        None,
    )
}

pub(in crate::serve) fn dedicated_token(
    configured: Option<String>,
    machine_bearer: Option<&str>,
    federation: Option<&crate::coordination::remote::Config>,
) -> Option<String> {
    configured
        .map(|token| token.trim().to_owned())
        .filter(|token| {
            !token.is_empty()
                && axum::http::HeaderValue::from_str(token)
                    .is_ok_and(|value| value.to_str().is_ok())
                && machine_bearer != Some(token)
                && federation
                    .is_none_or(|config| token != &config.ingress_token && token != &config.token)
        })
}
