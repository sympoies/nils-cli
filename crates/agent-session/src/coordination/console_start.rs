//! Owned child sessions (`session-coordination-v1`, "Owned child sessions").
//!
//! The daemon half answers `POST /sessions/{id}/console-start/v1` for a
//! managed session by asking the aggregator's
//! `POST /api/coordination/sessions/v1`, with the federation relay token and
//! the caller's exact identity, to create a session owned by the caller's
//! Console owner. The CLI half is `agent-session start --via-console`: it
//! reaches that route only through the private daemon endpoint and never
//! reads relay secrets.

use std::io::Read;
use std::path::Path;
use std::time::Duration;

use serde_json::{Map, Value, json};

use super::remote::{self, Config};
use crate::lineage::{self, LineageSeed, WorkRequest};
use crate::{CliContext, CliError};

pub(crate) mod diagnostics;
mod operation;
mod retention;
pub(crate) mod service;
mod start_args;
mod start_failure;
pub(crate) use start_args::{OperationArgs, render_text, start_args};

pub(crate) const RESULT_SCHEMA: &str = "agent-session.console-start.v1";
pub(crate) const DISABLED_CODE: &str = "console-start-disabled";
pub(crate) const UNAVAILABLE_CODE: &str = "console-start-unavailable";
const INVALID_CODE: &str = "console-start-invalid";
const MAX_BODY_BYTES: u64 = 1024 * 1024;
const MAX_MESSAGE_BYTES: usize = 256;
const MAX_CODE_BYTES: usize = 64;
/// A create that carries a prompt or selects an account can take the
/// aggregator and the target daemon more than a minute.
const REQUEST_TIMEOUT: Duration = Duration::from_secs(120);

fn disabled() -> CliError {
    CliError::runtime(
        DISABLED_CODE,
        "federation is not configured on this daemon",
        None,
    )
}

fn unavailable(
    stage: &str,
    cause: &str,
    message: &str,
    retryable: bool,
    safe_to_retry: bool,
) -> CliError {
    let (next_action, recovery_kind) = match (retryable, safe_to_retry) {
        (true, true) => ("retry_same_submission", "retry_same_submission"),
        (false, true) => ("inspect_relay_configuration", "repair_relay_configuration"),
        _ => ("inspect_console_before_retry", "inspect_console_state"),
    };
    CliError::runtime(
        UNAVAILABLE_CODE,
        message,
        Some(json!({
            "failure_stage": stage,
            "failure_cause": cause,
            "retryable": retryable,
            "next_action": next_action,
            "recovery": {"kind": recovery_kind, "safe_to_retry": safe_to_retry}
        })),
    )
}

fn invalid() -> CliError {
    CliError::usage(
        INVALID_CODE,
        "a console start takes operation_id with session fields, or operation_id with reconcile:true",
        None,
    )
}

/// What the caller asks for the child's lineage and work: by default the
/// caller is the parent and its program and issues are inherited.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct ChildLineage {
    pub(crate) no_parent: bool,
    pub(crate) work: WorkRequest,
    pub(crate) role: Option<String>,
}

impl Default for ChildLineage {
    fn default() -> Self {
        Self {
            no_parent: false,
            role: None,
            work: WorkRequest {
                inherit: true,
                ..WorkRequest::default()
            },
        }
    }
}

pub(super) fn client() -> Result<reqwest::blocking::Client, CliError> {
    reqwest::blocking::Client::builder()
        .timeout(REQUEST_TIMEOUT)
        .redirect(reqwest::redirect::Policy::none())
        .build()
        .map_err(|_| {
            unavailable(
                "http_client",
                "client_setup_failed",
                "the console start client is unavailable",
                true,
                true,
            )
        })
}

/// A bounded, single-line, non-empty diagnostic string, or `None`.
fn bounded_line(value: &Value) -> Option<&str> {
    let text = value.as_str()?.trim();
    (!text.is_empty() && text.len() <= MAX_MESSAGE_BYTES && !text.chars().any(char::is_control))
        .then_some(text)
}

/// A stable failure code shape: lowercase ASCII words joined by `-`.
fn is_code(code: &str) -> bool {
    !code.is_empty()
        && code.len() <= MAX_CODE_BYTES
        && code
            .bytes()
            .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'-')
        && !code.starts_with('-')
}

/// The JSON body of a response, read to at most `MAX_BODY_BYTES`.
pub(super) fn read_json(response: reqwest::blocking::Response) -> Option<Value> {
    let mut bytes = Vec::new();
    response
        .take(MAX_BODY_BYTES + 1)
        .read_to_end(&mut bytes)
        .ok()?;
    if bytes.len() as u64 > MAX_BODY_BYTES {
        return None;
    }
    serde_json::from_slice(&bytes).ok()
}

/// A failure with its own code and message when both have a safe shape,
/// classified by the coordination v1 exit codes.
fn forwarded(code: Option<&str>, message: &Value) -> CliError {
    let code = code
        .filter(|code| is_code(code))
        .unwrap_or(UNAVAILABLE_CODE);
    let message = bounded_line(message).unwrap_or("the console start request failed");
    match code {
        INVALID_CODE
        | "console-start-service-invalid"
        | "console-start-operation-conflict"
        | "console-start-operation-not-found"
        | "console-start-operation-expired"
        | "invalid-request"
        | "role-invalid"
        | "role-requires-root"
        | "lineage-invalid"
        | "lineage-parent-mismatch"
        | "work-ref-invalid"
        | "workspace-invalid"
        | "origin-invalid"
        | "launch-env-key-refused"
        | "launch-env-allowlist-invalid"
        | "launch-env-too-many"
        | "launch-env-value-invalid"
        | "launch-env-invalid"
        | "launch-env-key-duplicate"
        | "launch-env-assignment-invalid" => CliError::usage(code, message, None),
        "coordination-unauthorized"
        | "console-start-service-unauthorized"
        | "console-start-service-owner-denied"
        | "console-start-service-target-denied"
        | "console-start-service-expired"
        | "delegation-unauthorized"
        | "delegation-scope-denied"
        | "delegation-expired"
        | "delegation-revoked"
        | "delegation-incarnation-replaced"
        | "session-incarnation-conflict"
        | "ownership-unknown"
        | "machine-forbidden" => CliError::data(code, message, None),
        _ => CliError::runtime(code, message, None),
    }
}

/// The request body: optional `machine`, `no_parent`, `work`, and `role`, and a
/// required `session` object.
fn checked_request(body: &Value) -> Option<(Option<&str>, ChildLineage, &Map<String, Value>)> {
    let request = body.as_object()?;
    if request.keys().any(|key| {
        !matches!(
            key.as_str(),
            "machine" | "session" | "no_parent" | "work" | "role" | "operation_id"
        )
    }) {
        return None;
    }
    let machine = match request.get("machine") {
        None | Some(Value::Null) => None,
        Some(value) => Some(value.as_str().filter(|machine| !machine.is_empty())?),
    };
    let mut child = ChildLineage::default();
    match request.get("no_parent") {
        None | Some(Value::Null) => {}
        Some(value) => child.no_parent = value.as_bool()?,
    }
    match request.get("work") {
        None | Some(Value::Null) => {}
        Some(value) => child.work = WorkRequest::from_request_json(value).ok()?,
    }
    match request.get("role") {
        None | Some(Value::Null) => {}
        Some(value) => child.role = lineage::role_from_request(Some(value.as_str()?)).ok()?,
    }
    Some((machine, child, request.get("session")?.as_object()?))
}

/// The child's `lineage` and resolved `work`, from the caller's own record on
/// `relay_machine`. The aggregator checks the parent against the caller's
/// identity and forwards both to the target daemon.
fn child_session(
    relay_machine: &str,
    caller: &crate::SessionRecord,
    request: &ChildLineage,
    session: &Map<String, Value>,
) -> Result<Map<String, Value>, CliError> {
    let mut session = session.clone();
    session.remove("lineage");
    session.remove("work");
    session.remove("role");
    let (seed, work) = if request.no_parent {
        let mut seed = LineageSeed::root(
            relay_machine,
            lineage::STARTER_OPERATOR,
            lineage::VIA_CONSOLE,
        );
        // A role-less bound root attests a launch by a non-session launcher,
        // so a session passes its initiator only to a role-bound root.
        if request.role.is_some() {
            seed.set_forge_context(crate::forge_identity::context_of(caller)?.map(|context| {
                nils_common::forge_identity::session::LaunchContext {
                    role: None,
                    ..context
                }
            }));
        }
        (seed, request.work.resolve(None))
    } else {
        let seed = LineageSeed::child_of(
            relay_machine,
            relay_machine,
            caller,
            lineage::STARTER_SESSION,
            lineage::VIA_CONSOLE,
        )?;
        (seed, request.work.resolve(caller.work.as_ref()))
    };
    let mut origin = crate::origin::Fields::of(caller);
    origin.origin.parent = Some(crate::origin::Parent {
        session_id: caller.id.clone(),
        machine: relay_machine.to_owned(),
    });
    origin.workspace = match session.get("workspace") {
        Some(value) if !value.is_null() => Some(value.as_str().ok_or_else(invalid)?.to_owned()),
        _ => origin.workspace,
    };
    origin.validate()?;
    session.insert("origin".into(), json!(origin.origin));
    session.insert("workspace".into(), json!(origin.workspace));
    session.insert("lineage".to_string(), seed.to_create_json());
    if let Some(work) = work {
        session.insert(
            "work".to_string(),
            json!({ "program": work.program, "issues": work.issues, "inherited": work.inherited }),
        );
    }
    if let Some(role) = &request.role {
        session.insert("role".to_string(), json!(role));
    }
    Ok(session)
}

/// `POST /sessions/{id}/console-start/v1`: authenticate the exact current
/// session incarnation, then ask the aggregator to create the child session.
/// The aggregator decides the owner from the caller's grant; this route has no
/// way to name one. No lock is held across the network call.
pub(crate) fn relay_route(
    context: &CliContext,
    federation: Option<&Config>,
    session: &str,
    token: &str,
    body: &Value,
) -> Result<Value, CliError> {
    if body.get("operation_id").is_some_and(|id| !id.is_string()) {
        return Err(invalid());
    }
    let operation = body
        .get("operation_id")
        .and_then(Value::as_str)
        .map(str::to_owned)
        .unwrap_or_else(|| uuid::Uuid::now_v7().to_string());
    if !operation::valid_id(&operation) {
        return Err(invalid());
    }
    diagnostics::run(&operation, false, || {
        relay_operation(context, federation, session, token, body, &operation)
    })
}

fn relay_operation(
    context: &CliContext,
    federation: Option<&Config>,
    session: &str,
    token: &str,
    body: &Value,
    operation_id: &str,
) -> Result<Value, CliError> {
    let (caller, incarnation) = super::delegation::authenticate_token(
        context,
        Some(session),
        token,
        super::delegation::Use::StartViaConsole,
        true,
    )?;
    let Some(config) = federation else {
        return Err(disabled());
    };
    let scope = operation::Scope::Session {
        caller: session,
        incarnation: &incarnation,
    };
    if body["reconcile"] == true {
        if body
            .as_object()
            .is_none_or(|body| body.len() != 2 || !body.contains_key("operation_id"))
        {
            return Err(invalid());
        }
        let origin = operation::Origin::session(session, &incarnation, &caller.created_at, false);
        let (receipt, _) =
            operation::Receipt::begin(context, &origin, operation_id, body, &config.machine)?;
        return receipt.reconcile(config, &scope);
    }
    let (machine, lineage, child) = checked_request(body).ok_or_else(invalid)?;
    lineage::require_root_for_role(lineage.role.as_deref(), !lineage.no_parent)?;
    let mut child = child_session(&config.machine, &caller, &lineage, child)?;
    let origin =
        operation::Origin::session(session, &incarnation, &caller.created_at, lineage.no_parent);
    let (mut receipt, submit) = operation::Receipt::begin(
        context,
        &origin,
        operation_id,
        body,
        machine.unwrap_or(&config.machine),
    )?;
    if !submit {
        return receipt.reconcile(config, &scope);
    }
    let result = (|| {
        if child.contains_key("resume") {
            let request =
                super::resume_preflight::request_from_session(&Value::Object(child.clone()))?;
            let data = super::resume_preflight::relay_authenticated(
                config,
                session,
                &incarnation,
                machine.unwrap_or(&config.machine),
                &request,
            )?;
            child.insert(
                "resume_account_explicit".into(),
                json!(request.account.is_some()),
            );
            child.insert("claude_account".into(), data["account"].clone());
        }
        let source = json!({"source_session_id": session, "source_incarnation": incarnation});
        relay_create(config, source, machine, child, operation_id)
    })();
    if receipt.finish(context, &result).is_err() {
        return Err(persist_failed(&result));
    }
    result
}

/// The start outcome could not be recorded; a created session stays named.
fn persist_failed(result: &Result<Value, CliError>) -> CliError {
    let mut error = unavailable(
        "operation_receipt",
        "persist_failed",
        "the start outcome could not be recorded; reconcile before retrying",
        false,
        false,
    );
    if let Ok(value) = result {
        error.details_mut().as_mut().unwrap()["created_session_id"] =
            value["session"]["id"].clone();
    }
    error
}

/// `source` names who asks: the caller's session members, or an admitted
/// service's selectors and owner. Its members are merged into the request.
fn relay_create(
    config: &Config,
    source: Value,
    machine: Option<&str>,
    child: Map<String, Value>,
    operation_id: &str,
) -> Result<Value, CliError> {
    let mut request = json!({ "session": child });
    request
        .as_object_mut()
        .expect("request object")
        .extend(source.as_object().expect("source object").clone());
    if let Some(machine) = machine {
        request["machine"] = json!(machine);
    }
    let response = client()?
        .post(format!("{}/api/coordination/sessions/v1", config.url))
        .header("x-agent-session-operation-id", operation_id)
        .bearer_auth(&config.token)
        .json(&request)
        .send()
        .map_err(|_| {
            unavailable(
                "aggregator_request",
                "transport_failed",
                "the console aggregator request failed; inspect Console before retrying",
                false,
                false,
            )
        })?;
    let status = response.status();
    let body = read_json(response).ok_or_else(|| {
        unavailable(
            "aggregator_response",
            "response_unreadable",
            "the console aggregator response could not be read; inspect Console before retrying",
            false,
            false,
        )
    })?;
    if status.is_success() {
        let created = body.pointer("/data/session").filter(|_| body["ok"] == true);
        return match created {
            Some(created) if created["id"].is_string() => Ok(json!({
                "schema_version": RESULT_SCHEMA,
                "machine": machine.unwrap_or(&config.machine),
                "session": created,
            })),
            _ => Err(unavailable(
                "aggregator_response",
                "created_session_unconfirmed",
                "the console aggregator response did not confirm the created session; inspect Console before retrying",
                false,
                false,
            )),
        };
    }
    if status.as_u16() == 401 {
        return Err(unavailable(
            "aggregator_authentication",
            "relay_credential_rejected",
            "the console aggregator rejected this daemon's relay credential",
            false,
            true,
        ));
    }
    let error = &body["error"];
    if error["code"] == UNAVAILABLE_CODE {
        let message = bounded_line(&error["message"]).unwrap_or(
            "the console aggregator reported an unavailable start; inspect Console before retrying",
        );
        return Err(unavailable(
            "aggregator_response",
            "upstream_unavailable",
            message,
            false,
            false,
        ));
    }
    let mut failure = start_failure::forward(error)
        .unwrap_or_else(|| forwarded(error["code"].as_str(), &error["message"]));
    if matches!(
        failure.code(),
        "title-mode-unconfirmed" | "launch-env-unconfirmed"
    ) && let Some(id) = error["details"]["created_session_id"].as_str()
        && !id.is_empty()
        && id.len() <= 128
        && id
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_' | b'.'))
    {
        *failure.details_mut() = Some(json!({"created_session_id":id,"safe_to_retry":false}));
    }
    Err(failure)
}

fn cli_operation(
    context: &CliContext,
    capability_file: Option<&Path>,
    machine: Option<&str>,
    session: Value,
    lineage: &ChildLineage,
    options: &OperationArgs,
) -> Result<Value, CliError> {
    let operation_id = options
        .operation_id
        .clone()
        .unwrap_or_else(|| uuid::Uuid::now_v7().to_string());
    if !operation::valid_id(&operation_id) {
        return Err(invalid());
    }
    cli_request(
        context,
        capability_file,
        machine,
        session,
        lineage,
        options,
        &operation_id,
    )
}

/// The managed caller's runtime or delegation credential and its session id.
fn managed_caller(
    context: &CliContext,
    capability_file: Option<&Path>,
) -> Result<(String, String), CliError> {
    let unmanaged = || {
        CliError::usage(
            "console-start-unmanaged",
            "--via-console requires a managed caller, a worker delegation, or an admitted service",
            None,
        )
    };
    let runtime_caller = crate::non_empty_env("AGENT_SESSION_ID");
    if runtime_caller.is_none()
        && capability_file.is_none()
        && std::env::var_os(super::CAPABILITY_ENV).is_none_or(|path| path.is_empty())
    {
        return Err(unmanaged());
    }
    let token = super::capability_token_from_file(capability_file)?;
    let caller = if super::delegation::is_delegation(&token) {
        super::delegation::authenticate_token(
            context,
            None,
            &token,
            super::delegation::Use::StartViaConsole,
            false,
        )?
        .0
        .id
    } else {
        let caller = runtime_caller.ok_or_else(unmanaged)?;
        super::authenticate_token(context, &caller, &token)?;
        caller
    };
    Ok((token, caller))
}

fn cli_request(
    context: &CliContext,
    capability_file: Option<&Path>,
    machine: Option<&str>,
    session: Value,
    lineage: &ChildLineage,
    options: &OperationArgs,
    operation_id: &str,
) -> Result<Value, CliError> {
    let reconcile = options.reconcile;
    // A service presents its own credential to a service route; a managed
    // caller (or its delegation) to its own session's route.
    let service = options.service.admitted();
    let (token, caller) = match &service {
        Some(service) => (service::credential(service.credential_file)?, None),
        None => {
            let (token, caller) = managed_caller(context, capability_file)?;
            (token, Some(caller))
        }
    };
    let resuming = session.get("resume").is_some();
    let route = if resuming {
        "console-resume"
    } else {
        "console-start"
    };
    let mut url = remote::daemon_url(context).map_err(|_| {
        unavailable(
            "daemon_endpoint",
            "endpoint_unavailable",
            "the local agent-session daemon endpoint is unavailable",
            true,
            true,
        )
    })?;
    let service_route = ["coordination", "services", "console-start", "v1"];
    let session_route;
    let segments: &[&str] = match &caller {
        Some(caller) => {
            session_route = ["sessions", caller.as_str(), route, "v1"];
            &session_route
        }
        None => &service_route,
    };
    url.path_segments_mut()
        .map_err(|_| {
            unavailable(
                "daemon_endpoint",
                "endpoint_invalid",
                "the local agent-session daemon endpoint is invalid",
                true,
                true,
            )
        })?
        .pop_if_empty()
        .extend(segments);
    let requested_pinned = session["title_mode"] == "pinned";
    let requested_launch_env = crate::launch_env::from_json(session.get("launch_env"))?;
    let request = match &service {
        Some(service) => service::cli_body(service, machine, session, operation_id, reconcile)?,
        None => {
            let mut request = json!({ "session": session });
            if let Some(machine) = machine {
                request["machine"] = json!(machine);
            }
            if lineage.no_parent {
                request["no_parent"] = json!(true);
            }
            if let Some(work) = lineage.work.to_request_json() {
                request["work"] = work;
            }
            if let Some(role) = &lineage.role {
                request["role"] = json!(role);
            }
            request["operation_id"] = json!(operation_id);
            if reconcile {
                request = json!({"operation_id":operation_id,"reconcile":true});
            }
            request
        }
    };
    let response = client()?
        .post(url)
        .bearer_auth(&token)
        .json(&request)
        .send()
        .map_err(|_| {
            unavailable(
                "daemon_request",
                "transport_failed",
                "the local daemon request failed; inspect Console before retrying",
                false,
                false,
            )
        })?;
    let status = response.status();
    if resuming && matches!(status.as_u16(), 404 | 405) {
        return Err(crate::start_resume::refusal("resume-unsupported"));
    }
    let body = read_json(response).ok_or_else(|| {
        unavailable(
            "daemon_response",
            "response_unreadable",
            "the local daemon's console start answer is unreadable; inspect Console before retrying",
            false,
            false,
        )
    })?;
    if status.is_success() {
        if body["schema_version"] == RESULT_SCHEMA
            && body["reconciliation"].is_object()
            && (reconcile || body["reconciliation"]["state"] != "created")
        {
            return Ok(body);
        }
        if !requested_launch_env.is_empty()
            && body["session"]["id"].is_string()
            && body["session"]["launch_env"] != json!(requested_launch_env)
        {
            return Err(CliError::runtime(
                "launch-env-unconfirmed",
                "the created session did not confirm its launch env; inspect it before retrying",
                Some(json!({"created_session_id":body["session"]["id"],"safe_to_retry":false})),
            ));
        }
        if requested_pinned
            && body["session"]["id"].is_string()
            && (body["session"]["title_mode"] != "pinned"
                || body["session"]["display_revision"].as_u64().is_none())
        {
            return Err(CliError::runtime(
                "title-mode-unconfirmed",
                "the created session did not confirm pinned title mode; inspect it before retrying",
                Some(json!({"created_session_id":body["session"]["id"],"safe_to_retry":false})),
            ));
        }
        return (body["schema_version"] == RESULT_SCHEMA && body["session"]["id"].is_string())
            .then_some(body)
            .ok_or_else(|| {
                unavailable(
                    "daemon_response",
                    "result_unconfirmed",
                    "the console start answer did not confirm the created session; inspect Console before retrying",
                    false,
                    false,
                )
            });
    }
    let error = &body["error"];
    let mut failure = if error["code"] == UNAVAILABLE_CODE {
        unavailable_from_details(&error["message"], &error["details"])
    } else {
        start_failure::forward(error)
            .unwrap_or_else(|| forwarded(error["code"].as_str(), &error["message"]))
    };
    let details = failure.details_mut().get_or_insert_with(|| json!({}));
    diagnostics::copy(&error["details"], details, operation_id);
    if matches!(
        error["code"].as_str(),
        Some("title-mode-unconfirmed" | "launch-env-unconfirmed" | "console-start-unavailable")
    ) && let Some(id) = error["details"]["created_session_id"].as_str()
        && !id.is_empty()
        && id.len() <= 128
        && id
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_' | b'.'))
    {
        details["created_session_id"] = json!(id);
        details["safe_to_retry"] = json!(false);
    }
    Err(failure)
}

/// Re-project the local daemon's typed unavailable details through a fixed
/// allowlist. Older daemons and malformed detail objects fail closed.
fn unavailable_from_details(message: &Value, details: &Value) -> CliError {
    let classified = details["failure_stage"]
        .as_str()
        .zip(details["failure_cause"].as_str())
        .and_then(|(stage, cause)| {
            failure_policy(stage, cause).map(|policy| (stage, cause, policy))
        });
    let (stage, cause, (retryable, safe_to_retry)) =
        classified.unwrap_or(("daemon_response", "result_unconfirmed", (false, false)));
    let message = bounded_line(message).unwrap_or("the console start request failed");
    unavailable(stage, cause, message, retryable, safe_to_retry)
}

/// Retry policy is bound to a recognized stage/cause pair, not caller data.
fn failure_policy(stage: &str, cause: &str) -> Option<(bool, bool)> {
    match (stage, cause) {
        ("http_client", "client_setup_failed")
        | ("daemon_endpoint", "endpoint_unavailable" | "endpoint_invalid") => Some((true, true)),
        ("aggregator_authentication", "relay_credential_rejected") => Some((false, true)),
        ("operation_receipt", "persist_failed") => Some((false, false)),
        ("daemon_request" | "aggregator_request", "transport_failed")
        | ("daemon_response", "response_unreadable" | "result_unconfirmed")
        | (
            "aggregator_response",
            "response_unreadable" | "created_session_unconfirmed" | "upstream_unavailable",
        ) => Some((false, false)),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use pretty_assertions::assert_eq;

    #[test]
    fn unavailable_details_fail_closed_on_inconsistent_retry_policy() {
        for details in [
            json!({
                "failure_stage": "aggregator_response",
                "failure_cause": "result_unconfirmed",
                "retryable": true,
                "recovery": {"safe_to_retry": true}
            }),
            json!({
                "failure_stage": "unknown",
                "failure_cause": "result_unconfirmed",
                "retryable": true,
                "recovery": {"safe_to_retry": true}
            }),
            json!({
                "failure_stage": "aggregator_response",
                "failure_cause": "unknown",
                "retryable": true,
                "recovery": {"safe_to_retry": true}
            }),
            json!({
                "failure_cause": "result_unconfirmed",
                "retryable": true,
                "recovery": {"safe_to_retry": true}
            }),
        ] {
            let error = unavailable_from_details(&json!("untrusted"), &details);
            let details = error.details().expect("typed details");
            assert_eq!(details["retryable"], false);
            assert_eq!(details["recovery"]["safe_to_retry"], false);
            assert_eq!(details["next_action"], "inspect_console_before_retry");
        }
    }

    #[test]
    fn origin_console_start_transports_parent_and_workspace_across_hosts() {
        let parent: crate::SessionRecord = serde_json::from_value(json!({
            "schema_version":"agent-session.session.v1", "id":"parent", "agent":"codex", "mode":"interactive", "title":null,
            "cwd":".", "tmux_session":"fixture", "prompt_file":null, "log_file":null,
            "created_at":"2030-01-01T00:00:00Z", "updated_at":"2030-01-01T00:00:00Z", "workspace":"example-workspace"
        })).unwrap();
        for explicit in [None, Some("another-workspace")] {
            let mut session = json!({"agent":"codex"});
            if let Some(workspace) = explicit {
                session["workspace"] = json!(workspace);
            }
            let child = child_session(
                "parent-host",
                &parent,
                &ChildLineage::default(),
                session.as_object().unwrap(),
            )
            .unwrap();
            assert_eq!(
                child["origin"],
                json!({"parent":{"session_id":"parent","machine":"parent-host"}})
            );
            assert_eq!(child["workspace"], explicit.unwrap_or("example-workspace"));
            // The target has no local parent record; the payload is self-contained.
            let fields: crate::origin::Fields =
                serde_json::from_value(Value::Object(child)).unwrap();
            fields.validate().unwrap();
        }
    }

    #[test]
    fn console_root_inherits_the_launch_initiator_only_with_a_role() {
        let caller: crate::SessionRecord = serde_json::from_value(json!({
            "schema_version":"agent-session.session.v1", "id":"caller", "agent":"codex", "mode":"interactive", "title":null,
            "cwd":".", "tmux_session":"fixture", "prompt_file":null, "log_file":null,
            "created_at":"2030-01-01T00:00:00Z", "updated_at":"2030-01-01T00:00:00Z",
            "lineage": {
                "schema_version":"agent-session.session-lineage.v1", "machine":"caller-host", "parent":null,
                "root":{"machine":"caller-host", "session_id":"caller", "session_created_at":"2030-01-01T00:00:00Z"},
                "depth":0, "starter":{"kind":"operator", "via":"cli"},
                "forge_context":{"initiator":"operator", "role":null}
            }
        }))
        .unwrap();
        let session = json!({"agent":"codex"});
        let start = |role: Option<&str>| {
            let request = ChildLineage {
                no_parent: true,
                role: role.map(str::to_owned),
                work: WorkRequest::default(),
            };
            child_session(
                "caller-host",
                &caller,
                &request,
                session.as_object().unwrap(),
            )
            .unwrap()
        };
        // A role-less bound root attests a non-session launcher; a session
        // never produces one.
        assert_eq!(start(None)["lineage"].get("forge_context"), None);
        assert_eq!(
            start(Some("coordinator"))["lineage"]["forge_context"],
            json!({"initiator":"operator","role":null})
        );
    }

    #[test]
    fn requests_admit_only_machine_and_a_session_object() {
        let session = json!({"agent": "claude"});
        let with_machine = json!({"machine": "c8", "session": session});
        let (machine, lineage, child) = checked_request(&with_machine).expect("with machine");
        assert_eq!(
            (machine, lineage, child),
            (
                Some("c8"),
                ChildLineage::default(),
                session.as_object().unwrap()
            )
        );
        let default = json!({"session": session});
        let (machine, _, _) = checked_request(&default).expect("default");
        assert_eq!(machine, None);
        let root = json!({
            "session": session,
            "no_parent": true,
            "work": {"issues": [{"provider": "github", "repository": "a/b", "number": 1}], "inherit": false}
        });
        let (_, lineage, _) = checked_request(&root).expect("root");
        assert_eq!(
            lineage,
            ChildLineage {
                no_parent: true,
                role: None,
                work: WorkRequest::from_flags(None, &["a/b#1".to_string()], true).unwrap(),
            }
        );
        for body in [
            json!({"session": session, "principal": "other"}),
            json!({"machine": 7, "session": session}),
            json!({"machine": "", "session": session}),
            json!({"machine": "c8"}),
            json!({"session": "claude"}),
            json!({"session": session, "no_parent": "yes"}),
            json!({"session": session, "role": "invalid role"}),
            json!({"session": session, "role": 7}),
            json!({"session": session, "work": {"program": "a/b#1"}}),
            json!([]),
        ] {
            assert!(checked_request(&body).is_none(), "{body}");
        }
    }

    #[test]
    fn launch_env_forwarded_validation_preserves_usage_errors() {
        for code in [
            "launch-env-key-refused",
            "launch-env-allowlist-invalid",
            "launch-env-too-many",
            "launch-env-value-invalid",
            "launch-env-invalid",
        ] {
            let error = forwarded(Some(code), &json!("refused")).into_inner();
            assert_eq!(error.code, code);
            assert_eq!(error.exit_code, 64, "{code}");
        }
    }

    #[test]
    fn forwarded_failures_keep_only_safe_shapes() {
        for code in [
            "ownership-unknown",
            "machine-forbidden",
            "session-incarnation-conflict",
        ] {
            let error = forwarded(Some(code), &json!("refused")).into_inner();
            assert_eq!(
                (error.code.as_str(), error.message.as_str(), error.exit_code),
                (code, "refused", 65),
                "{code}"
            );
        }
        for code in [
            "role-invalid",
            "role-requires-root",
            "lineage-invalid",
            "lineage-parent-mismatch",
            "work-ref-invalid",
        ] {
            let error = forwarded(Some(code), &json!("refused")).into_inner();
            assert_eq!((error.code.as_str(), error.exit_code), (code, 64), "{code}");
        }
        let error = forwarded(Some("invalid-request"), &json!(null)).into_inner();
        assert_eq!(
            (error.code.as_str(), error.message.as_str(), error.exit_code),
            ("invalid-request", "the console start request failed", 64)
        );
        let error = forwarded(Some("agent-profile-unavailable"), &json!("no profile")).into_inner();
        assert_eq!(
            (error.code.as_str(), error.exit_code),
            ("agent-profile-unavailable", 1)
        );
        for code in [
            None,
            Some(""),
            Some("Bad"),
            Some("-lead"),
            Some("has space"),
        ] {
            let error = forwarded(code, &json!("line\u{1b}[2J")).into_inner();
            assert_eq!(
                (error.code.as_str(), error.message.as_str()),
                (UNAVAILABLE_CODE, "the console start request failed"),
                "{code:?}"
            );
        }
    }
}
