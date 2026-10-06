//! Native issue/PR timeline and review comment mutation by database id.
use super::github_write::{self as write, invalid};
use crate::backend::{BackendCall, BackendProgram};
use crate::cli::{BINARY, CommentCommand, CommentKind, GlobalFlags};
use crate::error::ForgeError;
use nils_common::cli_contract::{OutputFormat, schema_version_for};
use serde::{Deserialize, Serialize};
use std::ffi::OsString;
#[derive(Deserialize)]
struct EditedComment {
    id: u64,
    html_url: String,
    body: String,
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
    deleted: bool,
}
pub fn run(
    global: &GlobalFlags,
    command: CommentCommand,
    format: OutputFormat,
) -> Result<i32, ForgeError> {
    let op = match &command {
        CommentCommand::Edit(_) => "comment.edit",
        CommentCommand::Delete(_) => "comment.delete",
    };
    let ctx = write::target(global, op)?;
    let (id, kind, body) = match command {
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
            (args.id, args.kind, Some(body))
        }
        CommentCommand::Delete(args) => (args.id, args.kind, None),
    };
    let category = match kind {
        CommentKind::Issue => "issues",
        CommentKind::Review => "pulls",
    };
    let repo = ctx.repo.as_deref().expect("target repository");
    let mut argv: Vec<OsString> = vec!["api".into()];
    ctx.push_github_api_hostname(&mut argv);
    argv.extend([
        format!("repos/{repo}/{category}/comments/{id}").into(),
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
                body: edited.map(|c| c.body),
                deleted: body.is_none(),
            })
        },
    );
    drop(payload_file);
    result
}
