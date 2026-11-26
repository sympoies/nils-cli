//! Managed-origin admission and confirmation for Console-attested creates.
use reqwest::{StatusCode, blocking};
use serde_json::{Map, Value, json};

use crate::CliError;
use crate::forge_identity::CONSOLE_RELAY_INITIATOR;
use crate::serve::strict_json::Unique;
use nils_common::forge_identity::session::CONTEXT_KEY;

use super::{Config, RESULT_SCHEMA};

const INTENT: &str = "attest_role";
const INVALID_BODY: &str = "console-create-invalid-body";
const UNAVAILABLE: &str = "console-create-attestation-unavailable";
const ROLE: &str = "steward";

pub(crate) fn invalid_body() -> CliError {
    CliError::usage(INVALID_BODY, "invalid attested create request", None)
}

fn unexpected_intent(value: &Value) -> bool {
    match value {
        Value::Object(object) => object
            .iter()
            .any(|(key, value)| key.eq_ignore_ascii_case(INTENT) || unexpected_intent(value)),
        Value::Array(values) => values.iter().any(unexpected_intent),
        _ => false,
    }
}

/// Validate the opt-in before ordinary processing can discard caller fields.
pub(super) fn requested_id(body: &Value) -> Result<Option<&str>, CliError> {
    let Some(body) = body.as_object() else {
        return Ok(None);
    };
    if body.iter().any(|(key, value)| {
        key != INTENT && (key.eq_ignore_ascii_case(INTENT) || unexpected_intent(value))
    }) {
        return Err(invalid_body());
    }
    match body.get(INTENT) {
        None | Some(Value::Bool(false)) => return Ok(None),
        Some(Value::Bool(true)) => {}
        Some(_) => return Err(invalid_body()),
    }
    let session = body
        .get("session")
        .and_then(Value::as_object)
        .ok_or_else(invalid_body)?;
    let id = session
        .get("id")
        .and_then(Value::as_str)
        .ok_or_else(invalid_body)?;
    crate::validate_id(id).map_err(|_| invalid_body())?;
    if body.get("no_parent").is_some_and(|root| !root.is_boolean()) {
        return Err(invalid_body());
    }

    let lineage = session.get("lineage").filter(|lineage| !lineage.is_null());
    if [body, session]
        .into_iter()
        .chain(lineage.and_then(Value::as_object))
        .any(|object| {
            ["role", CONTEXT_KEY, "initiator", "attested_role"]
                .iter()
                .any(|key| object.contains_key(*key))
        })
    {
        return Err(CliError::usage(
            "identity_session_binding_untrusted",
            "forge launch context requires a verified launch owner",
            None,
        ));
    }
    if body.get("no_parent") != Some(&Value::Bool(true))
        || lineage.is_some_and(|lineage| !lineage.as_object().is_some_and(Map::is_empty))
    {
        return Err(CliError::usage(
            "role-requires-root",
            "attested create requires root intent and daemon-owned lineage",
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
    .any(|key| session.get(*key).is_some_and(|value| !value.is_null()))
    {
        return Err(CliError::usage(
            "resume-unsupported",
            "this resume request is not supported",
            None,
        ));
    }
    Ok(Some(id))
}

pub(super) fn unavailable() -> CliError {
    CliError::unavailable(
        UNAVAILABLE,
        "Console create attestation is unconfirmed; reconcile the existing operation",
        Some(json!({
            "retryable": false,
            "next_action": "inspect_console_before_retry",
            "recovery": {"kind": "inspect_console_state", "safe_to_retry": false}
        })),
    )
}

pub(super) fn discover(client: &blocking::Client, config: &Config) -> Result<(), CliError> {
    let response = client
        .get(format!(
            "{}/api/coordination/attested-create/v1",
            config.url
        ))
        .bearer_auth(&config.token)
        .send()
        .map_err(|_| unavailable())?;
    if response.status() != StatusCode::OK {
        return Err(unavailable());
    }
    let Unique(body) = super::read_body(response).ok_or_else(unavailable)?;
    if body["schema_version"] == "agent-session.console-attested-create-capability.v1"
        && body["ok"] == true
        && body["data"]["machine"] == config.machine
        && body["data"]["capability"] == "console-attested-create/v1"
    {
        return Ok(());
    }
    Err(unavailable())
}

pub(super) fn confirms(session: &Value, id: &str) -> bool {
    let context = &session["lineage"][CONTEXT_KEY];
    session["id"] == id
        && session["role"] == ROLE
        && context["initiator"] == CONSOLE_RELAY_INITIATOR
        && context["role"] == ROLE
        && context["attested_role"] == ROLE
}

pub(super) fn confirm(
    response: Result<blocking::Response, reqwest::Error>,
    machine: &str,
    id: &str,
) -> Result<Value, CliError> {
    let response = response.map_err(|_| unavailable())?;
    let status = response.status();
    let Unique(body) = super::read_body(response).ok_or_else(unavailable)?;
    let session = &body["data"]["session"];
    if status == StatusCode::CREATED && body["ok"] == true && confirms(session, id) {
        return Ok(json!({
            "schema_version": RESULT_SCHEMA,
            "machine": machine,
            "session": session,
        }));
    }
    Err(typed_refusal(status, &body).unwrap_or_else(unavailable))
}

fn typed_refusal(status: StatusCode, body: &Value) -> Option<CliError> {
    let error = &body["error"];
    let code = error["code"].as_str()?;
    if body["ok"] != false
        || !error["message"].is_string()
        || !(super::is_code(code) || code == "identity_session_binding_untrusted")
        || matches!(
            code,
            UNAVAILABLE | "console-start-unavailable" | "create-outcome-unknown"
        )
    {
        return None;
    }
    if matches!(
        code,
        INVALID_BODY
            | "identity_session_binding_untrusted"
            | "role-requires-root"
            | "resume-unsupported"
    ) && status != StatusCode::BAD_REQUEST
    {
        return None;
    }
    let message = if code == INVALID_BODY {
        "invalid attested create request"
    } else {
        "the Console relay refused the attested create request"
    };
    let failure = match status {
        StatusCode::BAD_REQUEST => CliError::usage(code, message, None),
        StatusCode::FORBIDDEN | StatusCode::UNPROCESSABLE_ENTITY => {
            CliError::data(code, message, None)
        }
        StatusCode::UNAUTHORIZED | StatusCode::NOT_FOUND | StatusCode::CONFLICT => {
            CliError::runtime(code, message, None)
        }
        StatusCode::SERVICE_UNAVAILABLE if code == "token-not-configured" => {
            CliError::unavailable(code, message, None)
        }
        StatusCode::INTERNAL_SERVER_ERROR if code == "start-failed" => {
            CliError::runtime(code, message, None)
        }
        _ => return None,
    };
    (crate::serve::error_response::status_for(&failure) == status).then_some(failure)
}
