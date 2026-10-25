//! Native issue/PR timeline and review comment mutation by database id.
use super::github_write::{self as write, invalid};
use crate::backend::{BackendCall, BackendProgram, BackendRunner};
use crate::cli::{BINARY, CommentCommand, CommentKind, GlobalFlags};
use crate::error::ForgeError;
use nils_common::cli_contract::{OutputFormat, schema_version_for};
use serde::{Deserialize, Serialize};
use std::ffi::OsString;
#[path = "comment_redact.rs"]
mod redact;
#[derive(Deserialize)]
struct EditedComment {
    id: u64,
    html_url: String,
    body: String,
    updated_at: String,
}
#[derive(Serialize)]
struct CommentPayload {
    provider: &'static str,
    repository: String,
    id: u64,
    kind: CommentKind,
    #[serde(skip_serializing_if = "Option::is_none")]
    url: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    body: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    updated_at: Option<String>,
    deleted: bool,
}
pub fn run(
    global: &GlobalFlags,
    command: CommentCommand,
    format: OutputFormat,
) -> Result<i32, ForgeError> {
    let command = match command {
        CommentCommand::Redact(args) => return redact::run(global, args, format),
        command => command,
    };
    let op = match &command {
        CommentCommand::Edit(_) => "comment.edit",
        CommentCommand::Delete(_) => "comment.delete",
        CommentCommand::Redact(_) => unreachable!(),
    };
    let ctx = write::target(global, op)?;
    let (id, kind, body, expected_updated_at) = match command {
        CommentCommand::Edit(args) => {
            let body = match args.body {
                Some(body) => body,
                None => write::text_file(
                    args.body_file
                        .as_deref()
                        .expect("clap requires body source"),
                    op,
                )?,
            };
            if body.trim().is_empty() {
                return Err(invalid(
                    op,
                    "body_missing_summary",
                    "comment body must not be empty",
                ));
            }
            write::guard_text(&body, "comment")?;
            (args.id, args.kind, Some(body), args.expected_updated_at)
        }
        CommentCommand::Delete(args) => (args.id, args.kind, None, None),
        CommentCommand::Redact(_) => unreachable!(),
    };
    let category = match kind {
        CommentKind::Issue => "issues",
        CommentKind::Review => "pulls",
    };
    let repo = ctx.repo.as_deref().expect("target repository");
    let endpoint = format!("repos/{repo}/{category}/comments/{id}");
    if body.is_some() && !global.dry_run {
        preflight(&ctx, &endpoint, id, expected_updated_at.as_deref(), op)?;
    }
    let mut argv: Vec<OsString> = vec!["api".into()];
    ctx.push_github_api_hostname(&mut argv);
    argv.extend([
        endpoint.into(),
        "--method".into(),
        if body.is_some() { "PATCH" } else { "DELETE" }.into(),
    ]);
    let payload_file = body
        .as_ref()
        .map(|body| {
            let json = serde_json::json!({"body": body}).to_string();
            write::payload_file(json.as_bytes(), op)
        })
        .transpose()?;
    if let Some(file) = &payload_file {
        argv.extend(["--input".into(), file.path().as_os_str().to_owned()]);
    }
    let result = write::execute(
        global,
        op,
        &ctx,
        BackendCall::new(BackendProgram::Gh, argv),
        format,
        |stdout| {
            let edited = if body.is_some() {
                let edited: EditedComment = serde_json::from_str(&stdout).map_err(|_| {
                    ForgeError::software(
                        schema_version_for(BINARY, op, 1),
                        "invalid comment mutation response",
                        None,
                    )
                })?;
                if edited.id != id || Some(&edited.body) != body.as_ref() {
                    return Err(invalid(
                        op,
                        "comment_response_mismatch",
                        "provider response does not match the edited comment",
                    ));
                }
                Some(edited)
            } else {
                None
            };
            Ok(CommentPayload {
                provider: "github",
                repository: repo.to_string(),
                id,
                kind,
                url: edited.as_ref().map(|c| c.html_url.clone()),
                updated_at: edited.as_ref().map(|c| c.updated_at.clone()),
                body: edited.map(|c| c.body),
                deleted: body.is_none(),
            })
        },
    );
    drop(payload_file);
    result
}

#[derive(Deserialize)]
struct Actor {
    login: String,
}
#[derive(Deserialize)]
struct CommentSnapshot {
    id: u64,
    user: Actor,
    updated_at: String,
}

// This is a read-then-write guard, not a provider atomic compare-and-swap.
// Another edit between this read and the PATCH can still be overwritten.
fn preflight(
    ctx: &crate::provider::ProviderContext,
    endpoint: &str,
    id: u64,
    expected_updated_at: Option<&str>,
    op: &str,
) -> Result<(), ForgeError> {
    fn read<T: serde::de::DeserializeOwned>(
        ctx: &crate::provider::ProviderContext,
        endpoint: &str,
        op: &str,
    ) -> Result<T, ForgeError> {
        let mut argv: Vec<OsString> = vec!["api".into()];
        ctx.push_github_api_hostname(&mut argv);
        argv.push(endpoint.into());
        // Keep the enclosing edit's write-identity scope for both reads.
        let output =
            crate::rate_limit::default_runner().run(&BackendCall::new(BackendProgram::Gh, argv))?;
        serde_json::from_str(&output.stdout).map_err(|_| {
            ForgeError::software(
                schema_version_for(BINARY, op, 1),
                "invalid comment preflight response",
                None,
            )
        })
    }
    let actor: Actor = read(ctx, "user", op)?;
    let current: CommentSnapshot = read(ctx, endpoint, op)?;
    if current.id != id {
        return Err(invalid(
            op,
            "comment_response_mismatch",
            "provider returned a different comment",
        ));
    }
    if actor.login.is_empty() || current.user.login.is_empty() || current.updated_at.is_empty() {
        return Err(invalid(
            op,
            "comment_snapshot_invalid",
            "comment preflight requires an authenticated author and updated_at",
        ));
    }
    if !actor.login.eq_ignore_ascii_case(&current.user.login) {
        return Err(invalid(
            op,
            "comment_author_mismatch",
            "refusing to edit a comment authored by another identity",
        ));
    }
    if expected_updated_at.is_some_and(|expected| expected != current.updated_at) {
        return Err(invalid(
            op,
            "comment_stale",
            "comment changed since the expected updated_at; read it again before editing",
        ));
    }
    Ok(())
}
