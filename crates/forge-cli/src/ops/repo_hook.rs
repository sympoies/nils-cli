//! `repo hook list|set-url|set-active|batch` — GitHub repository webhooks.
//!
//! Spec / ops: `cli.forge-cli.repo.hook.<action>.v1`. The engine lives in
//! [`super::webhook`]; this module resolves the repository and the command.
use nils_common::cli_contract::{OutputFormat, schema_version_for};

use super::github_write as write;
use super::webhook::{Action, HookTarget, Kind, execute};
use crate::cli::{BINARY, GlobalFlags, RepoHookCommand};
use crate::error::ForgeError;
use crate::provider::{Provider, detect_unscoped, git_remote_url};

pub fn run(
    global: &GlobalFlags,
    command: RepoHookCommand,
    format: OutputFormat,
) -> Result<i32, ForgeError> {
    if let RepoHookCommand::Batch(args) = command {
        // Rows name their own repositories.
        if global.repo.is_some() {
            return Err(write::invalid(
                "repo.hook.batch",
                "repo_conflict",
                "--file rows name their own repositories; omit --repo",
            ));
        }
        let ctx = detect_unscoped(global.provider_hint(), &global.remote, None, git_remote_url)?;
        if ctx.provider != Provider::GitHub {
            return Err(ForgeError::provider_unsupported(
                schema_version_for(BINARY, "error", 1),
                "this operation supports GitHub only",
                None,
            ));
        }
        return execute(
            global,
            &ctx,
            Kind::Repo,
            None,
            Action::Batch(args.file),
            format,
        );
    }
    let (op, action) = match command {
        RepoHookCommand::List => ("repo.hook.list", Action::List),
        RepoHookCommand::Redeliver(args) => ("repo.hook.redeliver", Action::Redeliver(args)),
        RepoHookCommand::SetUrl(args) => (
            "repo.hook.set-url",
            Action::SetUrl {
                hook: args.hook,
                from: args.from,
                to: args.to,
            },
        ),
        RepoHookCommand::SetActive(args) => (
            "repo.hook.set-active",
            Action::SetActive {
                hook: args.hook,
                to: args.active,
            },
        ),
        RepoHookCommand::Batch(_) => unreachable!("handled above"),
    };
    let ctx = write::target(global, op)?;
    let repo = ctx.repo.clone().unwrap_or_default();
    execute(
        global,
        &ctx,
        Kind::Repo,
        Some(HookTarget::Repo(repo)),
        action,
        format,
    )
}
