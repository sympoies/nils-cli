//! Managed caller client for the versioned origin start route.
use std::time::Duration;

use reqwest::StatusCode;
use serde_json::{Value, json};

use crate::{CliContext, CliError, cli::StartArgs};

use super::{
    super::{capability_token_from_file, delegation, remote},
    origin::DISCOVERY_TIMEOUT,
    refusal::Failure,
    reply::Reply,
    wire::{START_SCHEMA, StartRequest, has_capability},
};

pub(crate) fn run_start(context: &CliContext, args: &StartArgs) -> Result<Value, CliError> {
    let unavailable = || crate::start_resume::refusal("resume-move-unavailable");
    let caller = crate::non_empty_env("AGENT_SESSION_ID").ok_or_else(unavailable)?;
    let token = capability_token_from_file(None).map_err(|_| unavailable())?;
    if delegation::is_delegation(&token) {
        return Err(delegation::scope_denied());
    }
    let (_, incarnation) = delegation::authenticate_read_only(
        context,
        Some(&caller),
        &token,
        delegation::Use::StartViaConsole,
    )
    .map_err(|_| unavailable())?;
    let (mut url, origin_machine) = remote::daemon_endpoint(context).map_err(|_| unavailable())?;
    let origin_machine = origin_machine.ok_or_else(unavailable)?;

    let unsupported = || {
        Reply::failure(
            Failure::before_start(StatusCode::BAD_REQUEST, "resume-unsupported"),
            None,
        )
        .into_cli()
        .unwrap_err()
    };
    if args.console_operation.reconcile
        || args.prompt.is_some()
        || args.prompt_file.is_some()
        || args.prompt_stdin
        || args.id.is_some()
        || args.agent_profile.is_some()
        || !args.agent_args.is_empty()
        || !args.launch_env.is_empty()
        || args.title.is_some()
        || args.title_mode != "auto"
        || args.no_parent
        || args.role.is_some()
        || args.program.is_some()
        || !args.issues.is_empty()
        || args.no_inherit_work
        || args.workspace.is_some()
        || args.notice.notice_kind.is_some()
        || args.notice.notice_source.is_some()
        || args.notice.notice_id.is_some()
    {
        return Err(unsupported());
    }
    let operation = args
        .console_operation
        .operation_id
        .clone()
        .unwrap_or_else(|| uuid::Uuid::now_v7().to_string());
    let body = json!({
        "schema_version": START_SCHEMA,
        "operation_id": operation,
        "machine": args.machine,
        "caller": {"session_id": caller, "session_incarnation": incarnation},
        "session": {
            "move_id": args.move_id, "agent": args.agent.as_str(), "resume": args.resume,
            "account": args.account, "cwd": args.cwd.as_ref().and_then(|path| path.to_str())
        }
    })
    .to_string();
    let request = StartRequest::parse(body.as_bytes()).map_err(|_| unsupported())?;
    url.path_segments_mut()
        .map_err(|_| unavailable())?
        .pop_if_empty()
        .extend(["sessions", &caller, "console-resume-move", "v1"]);
    let client = reqwest::blocking::Client::builder()
        .no_proxy()
        .redirect(reqwest::redirect::Policy::none())
        .timeout(Duration::from_secs(125))
        .build()
        .map_err(|_| unavailable())?;
    let discovery = client
        .get(url.clone())
        .bearer_auth(&token)
        .timeout(DISCOVERY_TIMEOUT)
        .send()
        .map_err(|_| unavailable())?;
    if !has_capability(discovery, &origin_machine) {
        return Err(unavailable());
    }

    client
        .post(url)
        .bearer_auth(token)
        .header("content-type", "application/json")
        .body(body)
        .send()
        .ok()
        .and_then(|reply| Reply::from_response(reply, &request))
        .unwrap_or_else(|| Reply::unknown(&request.session.move_id))
        .into_cli()
}
