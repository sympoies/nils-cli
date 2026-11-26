//! Identity-scoped code-scanning alert state transitions.
use super::{api_argv, dry_run, parse, schema};
use crate::backend::{BackendCall, BackendProgram, BackendRunner};
use crate::cli::{BINARY, GlobalFlags, SecurityAlertDismissArgs, SecurityAlertReopenArgs};
use crate::envelope::emit_success;
use crate::error::ForgeError;
use crate::provider::ProviderContext;
use crate::rate_limit::default_runner;
use nils_common::cli_contract::{OutputFormat, schema_version_for};
use serde_json::json;

pub(super) fn dismiss(
    ctx: &ProviderContext,
    repo: &str,
    global: &GlobalFlags,
    args: SecurityAlertDismissArgs,
    format: OutputFormat,
) -> Result<i32, ForgeError> {
    let comment = std::fs::read_to_string(&args.comment_file).map_err(|_| {
        ForgeError::validation(
            schema(),
            "security_comment_invalid",
            "cannot read --comment-file as UTF-8",
            None,
        )
    })?;
    if comment.trim().is_empty() {
        return Err(ForgeError::validation(
            schema(),
            "security_comment_invalid",
            "--comment-file must not be blank",
            None,
        ));
    }
    transition(
        ctx,
        repo,
        global,
        args.number,
        "dismiss",
        Some((args.reason.as_str(), &comment)),
        format,
    )
}
pub(super) fn reopen(
    ctx: &ProviderContext,
    repo: &str,
    global: &GlobalFlags,
    args: SecurityAlertReopenArgs,
    format: OutputFormat,
) -> Result<i32, ForgeError> {
    transition(ctx, repo, global, args.number, "reopen", None, format)
}
fn transition(
    ctx: &ProviderContext,
    repo: &str,
    global: &GlobalFlags,
    number: u64,
    operation: &str,
    dismissal: Option<(&str, &str)>,
    format: OutputFormat,
) -> Result<i32, ForgeError> {
    let state = if dismissal.is_some() {
        "dismissed"
    } else {
        "open"
    };
    let mut argv = api_argv(ctx);
    argv.extend([
        "-X".into(),
        "PATCH".into(),
        format!("repos/{repo}/code-scanning/alerts/{number}").into(),
        "-f".into(),
        format!("state={state}").into(),
    ]);
    if let Some((reason, comment)) = dismissal {
        argv.extend([
            "-f".into(),
            format!("dismissed_reason={reason}").into(),
            "-f".into(),
            format!("dismissed_comment={comment}").into(),
        ]);
    }
    let call = BackendCall::new(BackendProgram::Gh, argv).with_host(ctx.provider, &ctx.host);
    let version = schema_version_for(BINARY, &format!("security.alerts.{operation}"), 1);
    if global.dry_run {
        return dry_run(version, ctx, call, format);
    }
    let output = default_runner().run(&call).map_err(|error| {
        let detail = error.detail().unwrap_or_default();
        if detail.contains("HTTP 404") {
            ForgeError::validation(schema(), "security_alert_not_found", "Alert not found or inaccessible; check the alert number, repository and token access", None)
        } else if detail.contains("HTTP 403") || detail.contains("HTTP 401") {
            ForgeError::validation(schema(), "security_permission_denied", "GitHub denied the request; check code scanning write permissions, repository archive status and security settings", None)
        } else { error }
    })?;
    let alert = parse(&output.stdout)?;
    if alert.get("number").and_then(|v| v.as_u64()) != Some(number)
        || alert.get("state").and_then(|v| v.as_str()) != Some(state)
    {
        return Err(ForgeError::software(
            schema(),
            "unexpected code-scanning alert response",
            None,
        ));
    }
    let payload = json!({"provider":"github", "host":ctx.host, "repo":repo, "number":number, "state":state, "dismissed_reason":alert.get("dismissed_reason"), "dismissed_comment":alert.get("dismissed_comment"), "url":alert.get("html_url")});
    Ok(emit_success(version, payload, format, |p| {
        println!(
            "{} alert #{}: {}",
            repo,
            number,
            p["state"].as_str().unwrap_or_default()
        )
    }))
}
