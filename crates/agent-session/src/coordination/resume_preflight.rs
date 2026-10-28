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

/// Never accept a capability guess, partial binding, or a response for another target.
pub(crate) fn checked_response(
    body: Value,
    success: bool,
    request: &Request,
    machine: Option<&str>,
) -> Result<Value, CliError> {
    if body["schema_version"] != PREFLIGHT_SCHEMA {
        return Err(refusal("resume-unsupported"));
    }
    if body["ok"] == false {
        if let Some(error) = forward_refusal(&body["error"]) {
            return Err(error);
        }
        return Err(unavailable());
    }
    let data = &body["data"];
    if !success
        || body["ok"] != true
        || data.as_object().is_none_or(|d| d.len() != 4)
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
        return Err(refusal("resume-unsupported"));
    }
    // Rebuild the bounded public envelope; never return upstream history or filesystem fields.
    Ok(
        json!({"agent":request.agent,"provider_session_id":request.resume,"machine":data["machine"],"account":data["account"]}),
    )
}

/// Only explicit terminal admission evidence is safe to carry through a create hop.
pub(crate) fn forward_refusal(error: &Value) -> Option<CliError> {
    if error["details"]["session_created"] != false || error["details"]["safe_to_retry"] != false {
        return None;
    }
    let code = match error["code"].as_str()? {
        "resume-preflight-unavailable" => return Some(unavailable()),
        "resume-history-missing" => "resume-history-missing",
        "resume-account-mismatch" => "resume-account-mismatch",
        "resume-source-live" => "resume-source-live",
        "resume-source-unknown" => "resume-source-unknown",
        "resume-unsupported" => "resume-unsupported",
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
        return Err(refusal("resume-unsupported"));
    }
    let success = response.status().is_success();
    let body = console_start::read_json(response).ok_or_else(|| refusal("resume-unsupported"))?;
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
    let config = config.ok_or_else(|| refusal("resume-unsupported"))?;
    let object = body
        .as_object()
        .ok_or_else(|| refusal("resume-unsupported"))?;
    if object
        .keys()
        .any(|k| !matches!(k.as_str(), "machine" | "session"))
    {
        return Err(refusal("resume-unsupported"));
    }
    let machine = match body.get("machine") {
        None => config.machine.as_str(),
        Some(Value::String(s)) if !s.is_empty() => s,
        _ => return Err(refusal("resume-unsupported")),
    };
    let request: Request = serde_json::from_value(body["session"].clone())
        .map_err(|_| refusal("resume-unsupported"))?;
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
        return Err(refusal("resume-unsupported"));
    }
    let success = response.status().is_success();
    let body = console_start::read_json(response).ok_or_else(|| refusal("resume-unsupported"))?;
    checked_response(body, success, request, Some(machine))
}

pub(crate) fn request_from_session(session: &Value) -> Result<Request, CliError> {
    let mut request = json!({});
    for key in [
        "agent",
        "resume",
        "cwd",
        "claude_account",
        "agent_profile",
        "agent_args",
    ] {
        if let Some(value) = session.get(key) {
            request[key] = value.clone();
        }
    }
    serde_json::from_value(request).map_err(|_| refusal("resume-unsupported"))
}
