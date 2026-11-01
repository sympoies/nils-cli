//! GitHub REST requests, with status-aware absence and no mutation replay.
use crate::backend::{BackendCall, BackendProgram, BackendRunner};
use crate::error::ForgeError;
use crate::provider::ProviderContext;
use nils_common::cli_contract::schema_version_for;
use serde_json::Value;
use std::ffi::OsString;
use std::time::Duration;

pub(super) fn call(ctx: &ProviderContext, path: &str, method: &str) -> BackendCall {
    let mut argv: Vec<OsString> = vec!["api".into()];
    ctx.push_github_api_hostname(&mut argv);
    argv.extend([
        path.into(),
        "--method".into(),
        method.into(),
        "--include".into(),
    ]);
    BackendCall::new(BackendProgram::Gh, argv).with_host(ctx.provider, &ctx.host)
}

pub(super) fn request(op: &str, call: &BackendCall) -> Result<Option<Value>, ForgeError> {
    let output = crate::rate_limit::default_runner()
        .run_raw_with_timeout(call, Some(Duration::from_secs(30)))?;
    // gh api --include supplies HTTP headers even for HTTP failures. A process
    // error or unparseable response is never evidence of absence.
    let response = output
        .stdout
        .split_once("\r\n\r\n")
        .or_else(|| output.stdout.split_once("\n\n"));
    let (status, body) = response
        .and_then(|(headers, body)| {
            let mut line = headers.lines().next()?.split_whitespace();
            if !line.next()?.starts_with("HTTP/") {
                return None;
            }
            Some((line.next()?.parse::<u16>().ok()?, body))
        })
        .ok_or_else(|| {
            failure(
                op,
                "release_response_invalid",
                "provider omitted a valid HTTP response; inspect the release before retrying",
                None,
            )
        })?;
    if status == 404 && !output.status_success {
        return Ok(None);
    }
    if !output.status_success || !(200..300).contains(&status) {
        return Err(failure(
            op,
            "release_request_failed",
            "GitHub release request failed; check access and retry the read before any further mutation",
            Some(status),
        ));
    }
    if status == 204 {
        return Ok(Some(Value::Null));
    }
    let value = serde_json::from_str(body).map_err(|_| {
        failure(
            op,
            "release_response_invalid",
            "provider returned invalid JSON; inspect the release before retrying",
            Some(status),
        )
    })?;
    Ok(Some(value))
}

pub(super) fn get(
    ctx: &ProviderContext,
    op: &str,
    path: &str,
) -> Result<Option<Value>, ForgeError> {
    request(op, &call(ctx, path, "GET"))
}

fn failure(op: &str, kind: &'static str, message: &str, status: Option<u16>) -> ForgeError {
    ForgeError::unavailable_with_details(
        schema_version_for("forge-cli", op, 1),
        kind,
        message,
        serde_json::json!({"http_status":status, "next_action":"inspect_release", "retryable":true}),
    )
}
