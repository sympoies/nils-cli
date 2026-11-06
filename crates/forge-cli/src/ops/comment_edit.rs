//! Edit issue and PR timeline comments through provider REST APIs.
use super::{github_write as write, gitlab_api};
use crate::backend::{BackendCall, BackendProgram, BackendRunner, DryRunPayload};
use crate::cli::{BINARY, GlobalFlags, TimelineCommentEditArgs};
use crate::envelope::emit_success;
use crate::error::ForgeError;
use crate::provider::{Provider, detect, git_remote_url};
use nils_common::cli_contract::{OutputFormat, schema_version_for};
use serde::{Deserialize, Serialize};
use std::ffi::OsString;

#[derive(Deserialize)]
struct Edited {
    id: u64,
    body: String,
}
#[derive(Serialize)]
struct Payload {
    provider: &'static str,
    #[serde(skip_serializing_if = "Option::is_none")]
    number: Option<u64>,
    id: u64,
    body: String,
}

pub fn run(
    global: &GlobalFlags,
    args: TimelineCommentEditArgs,
    format: OutputFormat,
    pr: bool,
) -> Result<i32, ForgeError> {
    let op = if pr {
        "pr.comment-edit"
    } else {
        "issue.comment-edit"
    };
    let ctx = detect(
        global.provider_hint(),
        &global.remote,
        global.repo.as_deref(),
        git_remote_url,
    )?;
    let repo = ctx.repo.as_deref().ok_or_else(|| {
        write::invalid(
            op,
            "repository_required",
            "a resolved repository is required",
        )
    })?;
    if ctx.provider == Provider::Local {
        return Err(ForgeError::provider_unsupported(
            schema_version_for(BINARY, op, 1),
            "comment editing supports GitHub and GitLab",
            None,
        ));
    }
    if ctx.provider == Provider::GitHub && args.parent.is_some() {
        return Err(write::invalid(
            op,
            "parent_unsupported",
            "--parent is only supported for GitLab",
        ));
    }
    let body = write::text_file(&args.body_file, op)?;
    if body.trim().is_empty() {
        return Err(write::invalid(
            op,
            "body_missing_summary",
            "comment body must not be empty",
        ));
    }
    crate::validations::no_local_path(&body, "comment")?;
    crate::validations::no_agent_attribution(&body, "comment")?;
    if !pr {
        crate::validations::no_escaped_control_markdown(&body)?;
    }
    let runner = crate::rate_limit::default_runner();
    let call = if ctx.provider == Provider::GitHub {
        let endpoint = format!("repos/{repo}/issues/comments/{}", args.comment_id);
        let mut argv: Vec<OsString> = vec!["api".into()];
        ctx.push_github_api_hostname(&mut argv);
        argv.push(endpoint.into());
        argv.extend(["--method".into(), "PATCH".into()]);
        BackendCall::new(BackendProgram::Gh, argv)
    } else {
        let parent = if pr { "merge_requests" } else { "issues" };
        let parent_id = args.parent.ok_or_else(|| {
            write::invalid(op, "parent_required", "--parent is required for GitLab")
        })?;
        let path = format!(
            "projects/{}/{parent}/{}/notes/{}",
            gitlab_api::encode_project_path(repo),
            parent_id,
            args.comment_id
        );
        gitlab_api::api_call_with_method_fields(&ctx.host, "PUT", path, &[])
    };
    let payload_file =
        write::payload_file(serde_json::json!({"body": body}).to_string().as_bytes(), op)?;
    let mut call = call;
    call.argv
        .extend(["--input".into(), payload_file.path().as_os_str().to_owned()]);
    if global.dry_run {
        return Ok(emit_success(
            schema_version_for(BINARY, op, 1),
            DryRunPayload::new(ctx.provider, &call),
            format,
            |p| println!("would run: {}", p.plan.join(" ")),
        ));
    }
    let output = runner.run(&call)?;
    let edited: Edited = serde_json::from_str(&output.stdout).map_err(|_| {
        ForgeError::software(
            schema_version_for(BINARY, op, 1),
            "invalid comment mutation response",
            None,
        )
    })?;
    if edited.id != args.comment_id || edited.body != body {
        return Err(write::invalid(
            op,
            "comment_response_mismatch",
            "provider response does not match the edited comment",
        ));
    }
    Ok(emit_success(
        schema_version_for(BINARY, op, 1),
        Payload {
            provider: ctx.provider.as_str(),
            number: args.parent,
            id: edited.id,
            body: edited.body,
        },
        format,
        |p| print!("{}", p.body),
    ))
}
