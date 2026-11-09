//! Read-only resume capability negotiation through the caller's owning Console.
use super::{console_start, delegation, remote};
use crate::start_resume::{PREFLIGHT_SCHEMA, Request, refusal};
use crate::{CliContext, CliError, cli};
use serde_json::{Value, json};

fn unavailable() -> CliError {
    CliError::runtime(
        "resume-preflight-unavailable",
        "the authenticated read-only resume check is unavailable",
        Some(json!({"session_created":false,"safe_to_retry":false})),
    )
}

// Only these static values may cross the preflight refusal boundary.
fn unsupported(reason: &'static str) -> CliError {
    let mut error = refusal("resume-unsupported");
    error.details_mut().as_mut().unwrap()["reason"] = json!(reason);
    error
}

fn known_reason(value: &Value) -> Option<&'static str> {
    match value.as_str()? {
        "daemon-route-missing" => Some("daemon-route-missing"),
        "daemon-answer-unreadable" => Some("daemon-answer-unreadable"),
        "preflight-schema-mismatch" => Some("preflight-schema-mismatch"),
        "preflight-binding-mismatch" => Some("preflight-binding-mismatch"),
        "relay-config-missing" => Some("relay-config-missing"),
        "relay-request-invalid" => Some("relay-request-invalid"),
        "console-route-missing" => Some("console-route-missing"),
        "console-answer-invalid" => Some("console-answer-invalid"),
        _ => None,
    }
}

/// Never accept a capability guess, partial binding, or a response for another target.
pub(crate) fn checked_response(
    body: Value,
    success: bool,
    request: &Request,
    machine: Option<&str>,
) -> Result<Value, CliError> {
    if body["schema_version"] != PREFLIGHT_SCHEMA {
        return Err(unsupported("preflight-schema-mismatch"));
    }
    if body["ok"] == false {
        if let Some(error) = forward_refusal(&body["error"]) {
            return Err(error);
        }
        return Err(unavailable());
    }
    let data = &body["data"];
    let known_shape =
        data.as_object()
            .is_some_and(|fields| match (fields.len(), fields.get("history_scope")) {
                (4, None) => true,
                (5, Some(scope)) => matches!(scope.as_str(), Some("account" | "shared")),
                _ => false,
            });
    if !success
        || body["ok"] != true
        || !known_shape
        || data["agent"] != request.agent
        || data["provider_session_id"] != request.resume
        || data["machine"].as_str().is_none_or(str::is_empty)
        || machine.is_some_and(|machine| data["machine"] != machine)
        || data["account"].as_str().is_none_or(|account| {
            !nils_common::provider_runtime::accounts::is_valid_account_nickname(account)
        })
        || request
            .account
            .as_ref()
            .is_some_and(|account| data["account"] != *account)
    {
        return Err(unsupported("preflight-binding-mismatch"));
    }
    // Rebuild the bounded public envelope; never return upstream history or filesystem fields.
    let mut result = json!({"agent":request.agent,"provider_session_id":request.resume,"machine":data["machine"],"account":data["account"]});
    if let Some(scope) = data.get("history_scope") {
        result["history_scope"] = scope.clone();
    }
    Ok(result)
}

/// Only explicit terminal admission evidence is safe to carry through a create hop.
pub(crate) fn forward_refusal(error: &Value) -> Option<CliError> {
    if error["details"]["session_created"] != false || error["details"]["safe_to_retry"] != false {
        return None;
    }
    let code = match error["code"].as_str()? {
        "resume-account-transition-pending" => {
            return Some(CliError::data(
                "resume-account-transition-pending",
                "select the bound account explicitly to abandon the pending switch, or resume the existing session to apply it",
                Some(json!({"session_created":false,"safe_to_retry":false})),
            ));
        }
        "resume-preflight-unavailable" => return Some(unavailable()),
        "resume-history-missing" => "resume-history-missing",
        "resume-account-mismatch" => "resume-account-mismatch",
        "resume-source-live" => "resume-source-live",
        "resume-source-unknown" => "resume-source-unknown",
        "resume-unsupported" => {
            return Some(match known_reason(&error["details"]["reason"]) {
                Some(reason) => unsupported(reason),
                None => refusal("resume-unsupported"),
            });
        }
        _ => return None,
    };
    Some(refusal(code))
}

pub(crate) fn check_args(context: &CliContext, args: &cli::StartArgs) -> Result<Value, CliError> {
    let mut request = Request::from_args(args);
    request.validate()?;
    if let Some(cwd) = &request.cwd {
        request.cwd = Some(crate::absolute_path(cwd)?);
    }
    let token = super::capability_token_from_file(None)?;
    let runtime_caller = crate::non_empty_env("AGENT_SESSION_ID");
    let (caller, _) = delegation::authenticate_read_only(
        context,
        runtime_caller.as_deref(),
        &token,
        delegation::Use::StartViaConsole,
    )?;
    let mut url = remote::daemon_url(context).map_err(|_| unavailable())?;
    url.path_segments_mut()
        .map_err(|_| unavailable())?
        .pop_if_empty()
        .extend(["sessions", caller.id.as_str(), "resume-preflight", "v1"]);
    let mut body = json!({"session":request});
    if let Some(machine) = &args.machine {
        body["machine"] = json!(machine);
    }
    let response = console_start::client()?
        .post(url)
        .bearer_auth(token)
        .json(&body)
        .send()
        .map_err(|_| unavailable())?;
    if matches!(response.status().as_u16(), 404 | 405) {
        return Err(unsupported("daemon-route-missing"));
    }
    let success = response.status().is_success();
    let body = console_start::read_json(response)
        .ok_or_else(|| unsupported("daemon-answer-unreadable"))?;
    checked_response(body, success, &request, args.machine.as_deref())
}

/// Authenticated source-incarnation binding without a registry maintenance or audit write.
pub(crate) fn relay_route(
    context: &CliContext,
    config: Option<&remote::Config>,
    caller: &str,
    token: &str,
    body: &Value,
) -> Result<Value, CliError> {
    let (_, incarnation) = delegation::authenticate_read_only(
        context,
        Some(caller),
        token,
        delegation::Use::StartViaConsole,
    )?;
    let config = config.ok_or_else(|| unsupported("relay-config-missing"))?;
    let object = body
        .as_object()
        .ok_or_else(|| unsupported("relay-request-invalid"))?;
    if object
        .keys()
        .any(|k| !matches!(k.as_str(), "machine" | "session"))
    {
        return Err(unsupported("relay-request-invalid"));
    }
    let machine = match body.get("machine") {
        None => config.machine.as_str(),
        Some(Value::String(s)) if !s.is_empty() => s,
        _ => return Err(unsupported("relay-request-invalid")),
    };
    let mut session = body["session"]
        .as_object()
        .cloned()
        .ok_or_else(|| unsupported("relay-request-invalid"))?;
    normalize_account(&mut session).map_err(|_| unsupported("relay-request-invalid"))?;
    let request: Request = serde_json::from_value(Value::Object(session))
        .map_err(|_| unsupported("relay-request-invalid"))?;
    relay_authenticated(config, caller, &incarnation, machine, &request)
}

pub(crate) fn relay_authenticated(
    config: &remote::Config,
    caller: &str,
    incarnation: &str,
    machine: &str,
    request: &Request,
) -> Result<Value, CliError> {
    request.validate()?;
    let body = json!({"source_session_id":caller,"source_incarnation":incarnation,"machine":machine,"session":request});
    let response = console_start::client()?
        .post(format!(
            "{}/api/coordination/resume-preflight/v1",
            config.url
        ))
        .bearer_auth(&config.token)
        .json(&body)
        .send()
        .map_err(|_| unavailable())?;
    if matches!(response.status().as_u16(), 404 | 405) {
        return Err(unsupported("console-route-missing"));
    }
    let success = response.status().is_success();
    let body =
        console_start::read_json(response).ok_or_else(|| unsupported("console-answer-invalid"))?;
    if body["schema_version"] != PREFLIGHT_SCHEMA {
        return Err(unsupported("console-answer-invalid"));
    }
    checked_response(body, success, request, Some(machine))
}

/// Accept the previous provider-specific request fields for one transition release.
/// Remove this normalization with nils-cli#3468.
pub(crate) fn normalize_account(
    session: &mut serde_json::Map<String, Value>,
) -> Result<(), CliError> {
    let codex = session.remove("codex_account");
    let claude = session.remove("claude_account");
    if codex.is_some() && claude.is_some()
        || session.contains_key("account") && (codex.is_some() || claude.is_some())
        || codex.is_some() && session.get("agent").and_then(Value::as_str) != Some("codex")
        || claude.is_some() && session.get("agent").and_then(Value::as_str) != Some("claude")
    {
        return Err(refusal("resume-unsupported"));
    }
    if let Some(account) = codex.or(claude) {
        session.insert("account".into(), account);
    }
    Ok(())
}

pub(crate) fn request_from_session(session: &Value) -> Result<Request, CliError> {
    let mut session = session
        .as_object()
        .cloned()
        .ok_or_else(|| refusal("resume-unsupported"))?;
    normalize_account(&mut session)?;
    let mut request = json!({});
    for key in [
        "agent",
        "resume",
        "cwd",
        "account",
        "agent_profile",
        "agent_args",
    ] {
        if let Some(value) = session.get(key) {
            request[key] = value.clone();
        }
    }
    serde_json::from_value(request).map_err(|_| refusal("resume-unsupported"))
}
