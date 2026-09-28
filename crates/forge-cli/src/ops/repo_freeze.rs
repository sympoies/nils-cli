//! `repo freeze start|end|status` — manage the GitHub merge-freeze record.
//!
//! Spec / ops: `cli.forge-cli.repo.freeze.v1`. A freeze is an open issue
//! labelled [`merge_policy::FREEZE_LABEL`]; `pr merge` refuses to merge while
//! one is open (`merge_freeze_active`). Keeping the record on the provider
//! makes it visible to every host, session, and human, and it works on private
//! repositories whose plan offers no branch protection or merge queue.

use std::ffi::OsString;

use nils_common::cli_contract::{OutputFormat, schema_version_for};
use serde::Serialize;

use crate::backend::{BackendCall, BackendProgram, BackendRunner};
use crate::cli::{BINARY, GlobalFlags, RepoFreezeCommand};
use crate::envelope::emit_success;
use crate::error::ForgeError;
use crate::ops::merge_policy::{self, FREEZE_LABEL, Freeze};
use crate::provider::{Provider, ProviderContext, detect, git_remote_url};
use crate::rate_limit::default_runner;

pub const SCHEMA: &str = "repo.freeze";
pub const SCHEMA_VERSION: u32 = 1;

const LABEL_COLOR: &str = "B60205";
const LABEL_DESCRIPTION: &str = "Open issue = active merge freeze enforced by forge-cli pr merge";

/// Envelope payload for `cli.forge-cli.repo.freeze.v1`.
#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct RepoFreezePayload {
    pub provider: &'static str,
    pub action: &'static str,
    /// Whether a freeze remains active after this command.
    pub active: bool,
    /// `status`: every active freeze. `start`: the new freeze. `end`: the
    /// freeze that was closed.
    pub freezes: Vec<Freeze>,
}

pub fn run(
    global: &GlobalFlags,
    command: RepoFreezeCommand,
    format: OutputFormat,
) -> Result<i32, ForgeError> {
    let runner = default_runner();
    let ctx = detect(
        global.provider_hint(),
        &global.remote,
        global.repo.as_deref(),
        git_remote_url,
    )?;
    let payload = compute(&runner, &ctx, command)?;
    Ok(emit_success(
        schema_version_for(BINARY, SCHEMA, SCHEMA_VERSION),
        payload,
        format,
        render_text,
    ))
}

pub fn compute<R: BackendRunner>(
    runner: &R,
    ctx: &ProviderContext,
    command: RepoFreezeCommand,
) -> Result<RepoFreezePayload, ForgeError> {
    if ctx.provider != Provider::GitHub {
        return Err(ForgeError::provider_unsupported(
            schema_err(),
            format!(
                "repo freeze is GitHub-only in v1 (provider: {})",
                ctx.provider.as_str()
            ),
            None,
        ));
    }
    match command {
        RepoFreezeCommand::Status => {
            let freezes = list_active(runner, ctx)?;
            Ok(payload("status", !freezes.is_empty(), freezes))
        }
        RepoFreezeCommand::Start { reason, until } => {
            ensure_label(runner, ctx)?;
            let freeze = open_freeze(runner, ctx, &reason, until.as_deref())?;
            Ok(payload("start", true, vec![freeze]))
        }
        RepoFreezeCommand::End { issue } => {
            let active = list_active(runner, ctx)?;
            let target = match issue {
                Some(number) => active
                    .iter()
                    .find(|freeze| freeze.number == number)
                    .cloned()
                    .ok_or_else(|| {
                        not_active(format!("issue #{number} is not an open merge freeze"))
                    })?,
                None => match active.as_slice() {
                    [] => return Err(not_active("no merge freeze is active".to_string())),
                    [only] => only.clone(),
                    many => {
                        return Err(ForgeError::validation(
                            schema_err(),
                            "merge_freeze_ambiguous",
                            "several merge freezes are active; pass --issue",
                            Some(
                                many.iter()
                                    .map(Freeze::describe)
                                    .collect::<Vec<_>>()
                                    .join("; "),
                            ),
                        ));
                    }
                },
            };
            close_freeze(runner, ctx, target.number)?;
            Ok(payload("end", active.len() > 1, vec![target]))
        }
    }
}

fn payload(action: &'static str, active: bool, freezes: Vec<Freeze>) -> RepoFreezePayload {
    RepoFreezePayload {
        provider: Provider::GitHub.as_str(),
        action,
        active,
        freezes,
    }
}

fn list_active<R: BackendRunner>(
    runner: &R,
    ctx: &ProviderContext,
) -> Result<Vec<Freeze>, ForgeError> {
    let output = runner.run(&gh_call(
        ctx,
        &[
            "issue",
            "list",
            "--label",
            FREEZE_LABEL,
            "--state",
            "open",
            "--limit",
            "50",
            "--json",
            "number,title,url,author,createdAt",
        ],
    ))?;
    let value: serde_json::Value = serde_json::from_str(output.stdout.trim()).map_err(|error| {
        ForgeError::software(
            schema_err(),
            "issue list returned invalid JSON",
            Some(error.to_string()),
        )
    })?;
    Ok(value
        .as_array()
        .map(|nodes| {
            nodes
                .iter()
                .filter_map(merge_policy::parse_freeze)
                .collect()
        })
        .unwrap_or_default())
}

fn ensure_label<R: BackendRunner>(runner: &R, ctx: &ProviderContext) -> Result<(), ForgeError> {
    let output = runner.run(&gh_call(
        ctx,
        &[
            "label",
            "list",
            "--search",
            FREEZE_LABEL,
            "--limit",
            "100",
            "--json",
            "name",
        ],
    ))?;
    let exists = serde_json::from_str::<serde_json::Value>(output.stdout.trim())
        .ok()
        .and_then(|value| value.as_array().cloned())
        .is_some_and(|labels| {
            labels
                .iter()
                .any(|label| label["name"].as_str() == Some(FREEZE_LABEL))
        });
    if !exists {
        runner.run(&gh_call(
            ctx,
            &[
                "label",
                "create",
                FREEZE_LABEL,
                "--color",
                LABEL_COLOR,
                "--description",
                LABEL_DESCRIPTION,
            ],
        ))?;
    }
    Ok(())
}

fn open_freeze<R: BackendRunner>(
    runner: &R,
    ctx: &ProviderContext,
    reason: &str,
    until: Option<&str>,
) -> Result<Freeze, ForgeError> {
    let title = format!("Merge freeze: {reason}");
    let body = format!(
        "Merge freeze recorded by `forge-cli repo freeze start`.\n\n\
         - Reason: {reason}\n\
         - Expected end: {until}\n\n\
         While this issue is open, `forge-cli pr merge` refuses to merge into this \
         repository (`merge_freeze_active`). End the freeze with \
         `forge-cli repo freeze end` or by closing this issue.\n",
        until = until.unwrap_or("not stated"),
    );
    let output = runner.run(&gh_call(
        ctx,
        &[
            "issue",
            "create",
            "--title",
            &title,
            "--body",
            &body,
            "--label",
            FREEZE_LABEL,
        ],
    ))?;
    let url = output
        .stdout
        .lines()
        .map(str::trim)
        .rfind(|line| line.starts_with("http"))
        .unwrap_or_default()
        .to_string();
    let number = url
        .rsplit('/')
        .next()
        .and_then(|segment| segment.parse::<u64>().ok())
        .ok_or_else(|| {
            ForgeError::software(
                schema_err(),
                "issue create did not return an issue URL",
                Some(format!("stdout={:?}", output.stdout)),
            )
        })?;
    Ok(Freeze {
        number,
        title,
        url,
        author: None,
        created_at: None,
    })
}

fn close_freeze<R: BackendRunner>(
    runner: &R,
    ctx: &ProviderContext,
    number: u64,
) -> Result<(), ForgeError> {
    let number = number.to_string();
    runner.run(&gh_call(
        ctx,
        &[
            "issue",
            "close",
            &number,
            "--comment",
            "Merge freeze ended by `forge-cli repo freeze end`.",
        ],
    ))?;
    Ok(())
}

fn gh_call(ctx: &ProviderContext, args: &[&str]) -> BackendCall {
    let mut argv: Vec<OsString> = args.iter().map(OsString::from).collect();
    ctx.push_repo_override(&mut argv);
    BackendCall::new(BackendProgram::Gh, argv)
}

fn not_active(message: String) -> ForgeError {
    ForgeError::validation(schema_err(), "merge_freeze_not_active", message, None)
}

fn schema_err() -> String {
    schema_version_for(BINARY, "error", 1)
}

fn render_text(payload: &RepoFreezePayload) {
    if payload.freezes.is_empty() {
        println!("no merge freeze is active");
        return;
    }
    for freeze in &payload.freezes {
        println!("{} {}", payload.action, freeze.describe());
    }
}
